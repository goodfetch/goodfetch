//! The engine's benchmark scenarios (ADR 0071), shared by its Callgrind and
//! DHAT targets: framing and decoding a fixed batch of lines, and
//! dispatching `hello` then a fixed number of `engine.info` calls over the
//! in-memory network on the tests' paused current-thread runtime, and
//! writing a fixed number of rows through a run file's writer, queued
//! together so they commit in one group, with SQLite's full sync, in a
//! temporary folder (ADR 0071's run-state writes). Inputs are literals; no
//! network, no sleep, no work whose amount depends on timing.

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::rpc::framing::{Frame, Framer, MAX_MESSAGE_BYTES, decode};
use goodfetch_engine::run_state::{RunState, RunStateOptions, RunStateTables};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const HELLO: &[u8] = br#"{"jsonrpc":"2.0","id":0,"method":"hello","params":{"protocol_version":"0.1","surface":"rust","needs":"collie","product_version":"0.0.0"}}"#;

/// `count` request lines of `engine.info`, ids 1 to `count`.
pub(crate) fn engine_info_lines(count: usize) -> Vec<u8> {
    let mut lines = Vec::new();
    for id in 1..=count {
        lines.extend_from_slice(
            format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"engine.info","params":{{}}}}"#)
                .as_bytes(),
        );
        lines.push(b'\n');
    }
    lines
}

/// Frames `bytes` and decodes each line; returns how many decoded.
pub(crate) fn frame_and_decode(bytes: &[u8]) -> usize {
    let mut framer = Framer::new(MAX_MESSAGE_BYTES);
    let mut lines = Vec::new();
    framer.feed(bytes, &mut lines);
    lines
        .iter()
        .filter(|frame| matches!(frame, Frame::Line(line) if decode(line).is_ok()))
        .count()
}

/// Opens a Connection to a Collie engine with only the engine's own
/// operations, sends `hello` and `calls` (lines from
/// [`engine_info_lines`]), and returns how many replies came back.
pub(crate) fn dispatch(calls: &[u8]) -> usize {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default().with(goodfetch_engine::contribution());
    let engine = Engine::new(identity, &composition).expect("invariant: one handler each");
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let send = async {
                writer.write_all(HELLO).await.expect("invariant: in memory");
                writer.write_all(b"\n").await.expect("invariant: in memory");
                writer.write_all(calls).await.expect("invariant: in memory");
                writer.shutdown().await.expect("invariant: in memory");
            };
            let receive = async {
                let mut lines = BufReader::new(reader).lines();
                let mut replies = 0;
                while let Ok(Some(_)) = lines.next_line().await {
                    replies += 1;
                }
                replies
            };
            tokio::join!(send, receive).1
        };
        let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
        let dependencies = Dependencies {
            home: Home::new(home.path().to_owned()),
            clock: Clock::paused(std::time::UNIX_EPOCH),
            entropy: Entropy::seeded(0),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
        };
        let (served, replies) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client
        );
        served.expect("invariant: the engine got its Connection");
        replies
    })
}

/// A table the run-state benchmark writes.
const ROWS: RunStateTables = RunStateTables {
    name: "bench",
    statements: &[
        "CREATE TABLE IF NOT EXISTS rows (value INTEGER NOT NULL, data TEXT NOT NULL) STRICT",
    ],
};

/// Opens a run file in a temporary folder, writes `count` rows through its
/// writer, queued together so they commit in one group, and returns how
/// many it acknowledged.
pub(crate) fn run_state_writes(count: usize) -> usize {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            folder.path().join("run.sqlite"),
            &RunStateOptions::default(),
            vec![ROWS],
            1,
            &owner,
        )
        .await
        .expect("invariant: a temporary folder takes a run file");
        let writes = (0..count).map(|value| {
            let state = state.clone();
            async move {
                state
                    .write(move |connection| {
                        connection.execute(
                            "INSERT INTO rows (value, data) VALUES (?1, ?2)",
                            (
                                i64::try_from(value).unwrap_or(i64::MAX),
                                "a Record of a few dozen bytes, as a crawl stages",
                            ),
                        )
                    })
                    .await
                    .is_ok()
            }
        });
        let acknowledged = join_all(writes)
            .await
            .into_iter()
            .filter(|acknowledged| *acknowledged)
            .count();
        drop(state);
        owner.wait().await;
        acknowledged
    })
}

/// Polls every future of `futures` on this task until each answered, so
/// the writes queue together.
async fn join_all<F: Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut pending: Vec<_> = futures.map(|future| Some(Box::pin(future))).collect();
    let mut answers: Vec<Option<F::Output>> = pending.iter().map(|_| None).collect();
    std::future::poll_fn(|context| {
        let mut waiting = false;
        for (slot, answer) in pending.iter_mut().zip(answers.iter_mut()) {
            if let Some(future) = slot {
                match future.as_mut().poll(context) {
                    std::task::Poll::Ready(output) => {
                        *answer = Some(output);
                        *slot = None;
                    }
                    std::task::Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    answers
        .into_iter()
        .map(|answer| answer.expect("invariant: every future answered"))
        .collect()
}
