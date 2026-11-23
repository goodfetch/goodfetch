//! The Daemon through the engine's public interface over the in-memory
//! network (spec #213's Seam 3), on the paused `Clock`: a `hello` without
//! its token closed unanswered, the idle stop counted on connected
//! Clients, what keeps it alive, each Client's Runs its own, `daemon.*`,
//! and the handover to its major's pick (ADRs 0024, 0078 and 0098).

use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::watch;

use std::collections::BTreeMap;

use goodfetch_protocol::run::{RunId, RunKind, Tags};
use tokio_util::sync::CancellationToken;

use super::run::{call, index_row, run_folder, run_row};
use super::{Client, attached_on, dependencies, engine, engine_with, records_of};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::daemon::{DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use crate::engine::{Dependencies, Engine};
use crate::ops::Ops;
use crate::runs::{EndedBy, Run, Start, Started};
use crate::tasks::BlockingPolicy;
use crate::transport::{self, InMemoryDialer, Network};

mod listener;

pub(super) const TOKEN: &str = "the-launch-token";

const MINUTE: Duration = Duration::from_secs(60);

/// `daemon.drain_grace`'s default.
const DRAIN_GRACE: Duration = Duration::from_secs(110);

/// A Succession whose answer the test fixes, counting the handovers begun.
struct Fixed {
    pick: bool,
    handovers: Handovers,
}

/// How many handovers began.
pub(super) type Handovers = Arc<watch::Sender<u32>>;

pub(super) fn no_handover() -> Handovers {
    Arc::new(watch::Sender::new(0))
}

fn begun(handovers: &Handovers) -> u32 {
    *handovers.borrow()
}

impl Succession for Fixed {
    fn is_pick(&self) -> bool {
        self.pick
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        self.handovers.send_modify(|count| *count += 1);
        Box::new("the start lock")
    }
}

pub(super) fn options(pick: bool, handovers: &Handovers) -> DaemonOptions {
    DaemonOptions {
        token: TOKEN.to_owned(),
        endpoint: DaemonEndpoint::Unix("daemon.sock".into()),
        pid: 4242,
        idle_timeout: 10 * MINUTE,
        keep_alive: false,
        holds: Holds::new(),
        succession: Arc::new(Fixed {
            pick,
            handovers: Arc::clone(handovers),
        }),
        drain: Drain {
            asked: None,
            grace: DRAIN_GRACE,
        },
        install: None,
        ops: Ops::default(),
        websocket: None,
    }
}

/// The pick's options.
pub(super) fn the_pick() -> DaemonOptions {
    options(true, &no_handover())
}

/// A Succession that is not the pick, whose handover, once begun, waits on
/// its blocking thread until the test lets it go.
struct Gated {
    begun: Handovers,
    #[expect(
        clippy::disallowed_types,
        reason = "it only makes the receiver shareable for the one handover that waits on it: no lock order to test under loom (ADR 0060)"
    )]
    go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Succession for Gated {
    fn is_pick(&self) -> bool {
        false
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        self.begun.send_modify(|count| *count += 1);
        let _ = self.go.lock().unwrap().recv();
        Box::new("the start lock")
    }
}

/// Runs `engine` as a Daemon with `options` in `env` while `body` dials it,
/// on the tests' paused runtime, and answers how it ended and what `body`
/// answered. As in `attached_with`, the test `tracing` subscriber writes the
/// dispatch records into the test's exercise record (ADR 0072).
pub(super) fn daemon<F, Fut, T>(
    engine: &Engine,
    env: &TestEnv,
    options: DaemonOptions,
    body: F,
) -> (DaemonEnd, T)
where
    F: FnOnce(InMemoryDialer) -> Fut,
    Fut: Future<Output = T>,
{
    on_daemon_runtime(
        env,
        BlockingPolicy::Inline,
        |dependencies, listener, dialer| async move {
            tokio::join!(
                engine.run_daemon(listener, dependencies, options),
                body(dialer)
            )
        },
    )
}

/// [`daemon`], its blocking work under `blocking`, its end none when it did
/// not end within `bound` on its `Clock`.
pub(super) fn daemon_within<F, Fut, T>(
    engine: &Engine,
    env: &TestEnv,
    blocking: BlockingPolicy,
    options: DaemonOptions,
    bound: Duration,
    body: F,
) -> (Option<DaemonEnd>, T)
where
    F: FnOnce(InMemoryDialer) -> Fut,
    Fut: Future<Output = T>,
{
    on_daemon_runtime(env, blocking, |dependencies, listener, dialer| async move {
        let clock = dependencies.clock;
        let ended = async {
            tokio::select! {
                end = engine.run_daemon(listener, dependencies, options) => Some(end),
                () = clock.sleep(bound) => None,
            }
        };
        tokio::join!(ended, body(dialer))
    })
}

/// Runs what `daemon` makes of the engine's dependencies in `env`, its
/// blocking work under `blocking`, a listener on the in-memory network and
/// its dialer, on the tests' paused runtime, with the test `tracing`
/// subscriber.
fn on_daemon_runtime<F, Fut, T>(env: &TestEnv, blocking: BlockingPolicy, daemon: F) -> T
where
    F: FnOnce(Dependencies, transport::DaemonListener, InMemoryDialer) -> Fut,
    Fut: Future<Output = T>,
{
    goodfetch_test_support::runtime().block_on(async {
        let dependencies = Dependencies {
            blocking,
            ..dependencies(env)
        };
        let clock = dependencies.clock;
        let _subscriber = goodfetch_test_support::subscriber_unless_own(move || clock.wall());
        let (network, dialer) = transport::in_memory();
        let (listener, _) = Network::InMemory(network)
            .listen(std::path::Path::new("daemon.sock"))
            .unwrap();
        daemon(dependencies, listener, dialer).await
    })
}

pub(super) async fn connect(dialer: &InMemoryDialer) -> Client {
    let (reader, writer) = dialer.connect().await.unwrap().split();
    Client {
        lines: BufReader::new(reader).lines(),
        writer,
    }
}

pub(super) fn hello_with(
    product: &str,
    protocol_version: &str,
    surface: &str,
    token: Option<&str>,
) -> Value {
    let mut params = json!({
        "protocol_version": protocol_version,
        "surface": surface,
        "needs": product,
        "product_version": "1.4.0",
    });
    if let Some(token) = token {
        params["token"] = json!(token);
    }
    json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": params })
}

pub(super) fn cli_hello() -> Value {
    hello_with("collie", "0.1", "cli", Some(TOKEN))
}

/// The `hello` of an SDK program's Connection, whose Runs end with it
/// (ADR 0078).
pub(super) fn sdk_hello() -> Value {
    hello_with("collie", "0.1", "sdk_typescript", Some(TOKEN))
}

/// The `hello` of a Kennel's Connection, which asks the Daemon to count it
/// as a Client that keeps it alive when `keep_daemon_alive` (ADR 0141).
pub(super) fn kennel_hello(keep_daemon_alive: bool) -> Value {
    let mut hello = hello_with("collie", "0.1", "kennel", Some(TOKEN));
    if keep_daemon_alive {
        hello["params"]["keep_daemon_alive"] = json!(true);
    }
    hello
}

/// A Client that said `hello` with the token.
pub(super) async fn opened(dialer: &InMemoryDialer, hello: Value) -> Client {
    let mut client = connect(dialer).await;
    client.send(&[hello]).await;
    let answer = client.next().await.unwrap();
    assert_eq!(answer["result"]["product"], json!("collie"), "{answer}");
    client
}

/// Whether the Daemon still answers a new Client.
async fn answers(dialer: &InMemoryDialer) -> bool {
    let Ok(connection) = dialer.connect().await else {
        return false;
    };
    let (reader, writer) = connection.split();
    let mut client = Client {
        lines: BufReader::new(reader).lines(),
        writer,
    };
    client.send(&[cli_hello()]).await;
    let answered = client.next().await.is_some();
    client.close().await;
    answered
}

/// Covers: story:3-11, story:3-12
#[test]
fn a_daemon_answers_a_hello_that_carries_its_token() {
    let (_, answer) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut client = connect(&dialer).await;
            client.send(&[cli_hello()]).await;
            client.next().await
        },
    );
    let mut answer = answer.unwrap();
    let named = super::takes_engine_instance(&mut answer);
    let timed = super::takes_timeouts(&mut answer);

    assert_eq!(
        (answer, named, timed),
        (
            json!({
                "jsonrpc": "2.0", "id": 0,
                "result": {
                    "protocol_version": "0.1", "product": "collie", "product_version": "1.4.0",
                    "max_calls_in_flight": super::IN_FLIGHT_CAP,
                },
            }),
            true,
            true
        )
    );
}

/// Covers: story:3-12
#[rstest]
#[case::no_token(hello_with("collie", "0.1", "cli", None))]
#[case::another_token(hello_with("collie", "0.1", "cli", Some("a-guessed-token")))]
#[case::a_request_before_hello(json!({ "jsonrpc": "2.0", "id": 0, "method": "engine.info", "params": {} }))]
#[case::a_notification_before_hello(json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 0 } }))]
#[case::a_hello_without_params(json!({ "jsonrpc": "2.0", "id": 0, "method": "hello" }))]
fn a_daemon_closes_unanswered_a_connection_that_does_not_open_with_hello_and_its_token(
    #[case] first: Value,
) {
    let (_, answer) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut client = connect(&dialer).await;
            client
                .send(&[
                    first,
                    json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {} }),
                ])
                .await;
            client.next().await
        },
    );

    assert_eq!(answer, None);
}

/// Covers: story:3-18, setting:daemon.idle_timeout
#[test]
fn a_daemon_stops_once_no_client_was_connected_for_its_idle_timeout() {
    let (end, (before, after)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            opened(&dialer, cli_hello()).await.close().await;
            tokio::time::sleep(9 * MINUTE).await;
            let before = answers(&dialer).await;
            tokio::time::sleep(11 * MINUTE).await;
            (before, answers(&dialer).await)
        },
    );

    assert_eq!(
        (before, after, format!("{end:?}")),
        (true, false, "Idle".to_owned())
    );
}

/// A Client keeps the Daemon past its idle timeout while it is connected
/// (ADR 0024).
///
/// Covers: story:3-18, story:3-19
#[rstest]
#[case::the_cli("cli")]
#[case::a_script("sdk_typescript")]
fn a_connected_client_keeps_the_daemon_past_its_idle_timeout(#[case] surface: &str) {
    let (end, info) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut client =
                opened(&dialer, hello_with("collie", "0.1", surface, Some(TOKEN))).await;
            tokio::time::sleep(60 * MINUTE).await;
            let info = call(&mut client, 1, "engine.info", json!({})).await;
            client.close().await;
            info["result"]["product"].clone()
        },
    );

    assert_eq!(
        (info, format!("{end:?}")),
        (json!("collie"), "Idle".to_owned())
    );
}

/// A Kennel only observes the Daemon (ADR 0141): with its Connection the
/// only one and no Run live, the Daemon stops at its idle timeout. A Kennel
/// whose `hello` asks to keep the Daemon alive holds it past its idle
/// timeout as any other Client does.
///
/// Covers: story:3-18, setting:daemon.idle_timeout
#[rstest]
#[case::observing(false, false)]
#[case::keeping_it_alive(true, true)]
fn a_kennel_holds_off_the_idle_stop_only_when_it_asks_to_keep_the_daemon_alive(
    #[case] keep_daemon_alive: bool,
    #[case] answered: bool,
) {
    let (end, after_the_timeout) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let kennel = opened(&dialer, kennel_hello(keep_daemon_alive)).await;
            tokio::time::sleep(11 * MINUTE).await;
            let answered = answers(&dialer).await;
            drop(kennel);
            answered
        },
    );

    assert_eq!(
        (after_the_timeout, format!("{end:?}")),
        (answered, "Idle".to_owned())
    );
}

/// Covers: story:3-18
#[test]
fn a_kept_alive_daemon_never_stops_idle_and_stops_when_asked() {
    let options = DaemonOptions {
        keep_alive: true,
        ..the_pick()
    };

    let (end, (answered, stopping)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        options,
        |dialer| async move {
            tokio::time::sleep(24 * 60 * MINUTE).await;
            let answered = answers(&dialer).await;
            let mut client = opened(&dialer, cli_hello()).await;
            let stopping = call(&mut client, 1, "daemon.stop", json!({})).await;
            client.close().await;
            (answered, stopping)
        },
    );

    assert_eq!(
        (answered, stopping["result"].clone(), format!("{end:?}")),
        (true, json!({ "live_runs": 0 }), "Stopped".to_owned())
    );
}

/// `daemon.stop` from one Client closes the Daemon's other Connections:
/// a call in flight on one answers `call_cancelled` naming the stopping
/// Client, or the person a person's Client acts for, the OS user of an
/// engine without accounts, and the Connection closed under it (ADR 0113).
///
/// Covers: story:11-48, operation:daemon.stop
#[rstest]
#[case::by_a_persons_client(
    cli_hello(),
    json!({ "by": "person", "closed": "connection", "person": "os_user" })
)]
#[case::by_a_programs_client(sdk_hello(), json!({ "by": "client", "closed": "connection" }))]
fn a_stop_from_another_client_cancels_a_call_in_flight_naming_its_connection(
    #[case] stopper: Value,
    #[case] details: Value,
) {
    let (_, (stopping, answered)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut waiting = opened(&dialer, sdk_hello()).await;
            waiting.send(&[super::request(1, "test.wait")]).await;
            // Read in order, so `test.wait` is in flight once this answers.
            call(&mut waiting, 2, "daemon.status", json!({})).await;
            let mut asking = opened(&dialer, stopper).await;
            let stopping = call(&mut asking, 1, "daemon.stop", json!({})).await;
            let answered = waiting.next().await;
            asking.close().await;
            (stopping, answered)
        },
    );

    let answered = answered.unwrap_or_default();
    let mut data = answered["error"]["data"].clone();
    if let Some(data) = data.as_object_mut() {
        data.remove("retryable");
    }
    let mut expected = details;
    expected["code"] = json!("call_cancelled");
    assert_eq!(
        (&stopping["result"], &answered["id"], &data),
        (&json!({ "live_runs": 0 }), &json!(1), &expected),
        "{answered}"
    );
}

/// A Schedule's hold keeps the Daemon past its idle timeout until it drops
/// (ADR 0036).
///
/// Covers: story:3-18
#[test]
fn a_hold_keeps_the_daemon_running_until_it_drops() {
    let holds = Holds::new();
    let held = holds.hold();
    let options = DaemonOptions {
        holds,
        ..the_pick()
    };

    let (end, (while_held, after)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        options,
        |dialer| async move {
            tokio::time::sleep(60 * MINUTE).await;
            let while_held = answers(&dialer).await;
            drop(held);
            tokio::time::sleep(11 * MINUTE).await;
            (while_held, answers(&dialer).await)
        },
    );

    assert_eq!(
        (while_held, after, format!("{end:?}")),
        (true, false, "Idle".to_owned())
    );
}

/// Covers: story:3-24, operation:daemon.status
#[test]
fn daemon_status_counts_its_clients_and_live_runs() {
    let (_, status) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut first = opened(&dialer, cli_hello()).await;
            let mut second = opened(&dialer, sdk_hello()).await;
            call(&mut first, 1, "run.start", json!({ "run_id": "one" })).await;
            call(&mut second, 1, "run.start", json!({ "run_id": "two" })).await;
            let status = call(&mut first, 2, "daemon.status", json!({})).await;
            // A CLI command's Run outlives its Connection and holds the
            // Daemon from its idle stop, so it ends before the test does.
            call(&mut first, 3, "run.finish", json!({ "run_id": "one" })).await;
            first.close().await;
            second.close().await;
            status
        },
    );

    assert_eq!(
        status["result"],
        json!({
            "product": "collie", "product_version": "1.4.0", "pid": 4242,
            "endpoint": { "unix": "daemon.sock" }, "clients": 2, "live_runs": 2,
            "kept_alive": false, "idle_timeout": 600_000,
        })
    );
}

/// A Client's crash stops only the Runs it started; another Client's go on
/// (ADR 0078).
///
/// Covers: story:3-23, story:3-24
#[test]
fn the_end_of_one_clients_connection_stops_only_its_runs() {
    let env = TestEnv::new();

    let (_, live) = daemon(
        &engine(Product::Collie),
        &env,
        the_pick(),
        |dialer| async move {
            let mut leaving = opened(&dialer, sdk_hello()).await;
            let mut staying = opened(&dialer, sdk_hello()).await;
            call(&mut leaving, 1, "run.start", json!({ "run_id": "left" })).await;
            call(&mut staying, 1, "run.start", json!({ "run_id": "stays" })).await;
            leaving.close().await;
            while leaving.next().await.is_some() {}
            tokio::time::sleep(MINUTE).await;
            let status = call(&mut staying, 2, "daemon.status", json!({})).await;
            staying.close().await;
            status["result"]["live_runs"].clone()
        },
    );

    assert_eq!(
        (
            live,
            index_row(&env, "left")["lifecycle"].clone(),
            index_row(&env, "stays")["lifecycle"].clone()
        ),
        (json!(1), json!("stopped"), json!("stopped"))
    );
}

/// Each CLI command is a Connection of its own, so the Runs a CLI
/// Connection starts outlive it: the next command finds them live (ADRs
/// 0024 and 0088).
///
/// Covers: story:11-199
#[test]
fn the_runs_a_cli_connection_starts_outlive_it() {
    let env = TestEnv::new();

    let (_, live) = daemon(
        &engine(Product::Collie),
        &env,
        the_pick(),
        |dialer| async move {
            let mut starting = opened(&dialer, cli_hello()).await;
            call(
                &mut starting,
                1,
                "run.start",
                json!({ "run_id": "lasting" }),
            )
            .await;
            starting.close().await;
            while starting.next().await.is_some() {}
            tokio::time::sleep(MINUTE).await;
            let mut next = opened(&dialer, cli_hello()).await;
            let status = call(&mut next, 1, "daemon.status", json!({})).await;
            call(&mut next, 2, "run.finish", json!({ "run_id": "lasting" })).await;
            next.close().await;
            status["result"]["live_runs"].clone()
        },
    );

    assert_eq!(
        (live, index_row(&env, "lasting")["lifecycle"].clone()),
        (json!(1), json!("finished"))
    );
}

/// A Run a person resumes from the Kennel outlives the Kennel's
/// Connection, as a CLI command's Runs do (ADR 0141): the next Client finds
/// it live.
///
/// Covers: story:11-199, operation:run.resume
#[test]
fn a_run_a_kennel_connection_resumes_outlives_it() {
    let env = TestEnv::new();

    let (_, live) = daemon(
        &engine(Product::Collie),
        &env,
        the_pick(),
        |dialer| async move {
            let mut starting = opened(&dialer, cli_hello()).await;
            call(&mut starting, 1, "run.start", json!({ "run_id": "paused" })).await;
            call(&mut starting, 2, "run.pause", json!({ "run_id": "paused" })).await;
            starting.close().await;
            while starting.next().await.is_some() {}
            let mut kennel = opened(&dialer, kennel_hello(false)).await;
            call(&mut kennel, 1, "run.resume", json!({ "run_id": "paused" })).await;
            kennel.close().await;
            while kennel.next().await.is_some() {}
            tokio::time::sleep(MINUTE).await;
            let mut next = opened(&dialer, cli_hello()).await;
            let status = call(&mut next, 1, "daemon.status", json!({})).await;
            call(&mut next, 2, "run.finish", json!({ "run_id": "paused" })).await;
            next.close().await;
            status["result"]["live_runs"].clone()
        },
    );

    assert_eq!(
        (live, index_row(&env, "paused")["lifecycle"].clone()),
        (json!(1), json!("finished"))
    );
}

/// A Run that outlives the CLI command that started it keeps the Daemon
/// from its idle stop until it ends, with no Client connected, so a crawl
/// started from the CLI runs to its end; once it ended, the Daemon stops
/// idle as before (608-O1).
///
/// Covers: story:3-18, story:11-199, setting:daemon.idle_timeout
#[test]
fn a_run_that_outlived_its_cli_command_keeps_the_daemon_until_it_ends() {
    let env = TestEnv::new();

    let (end, (live, after)) = daemon(
        &engine(Product::Collie),
        &env,
        the_pick(),
        |dialer| async move {
            let mut starting = opened(&dialer, cli_hello()).await;
            call(
                &mut starting,
                1,
                "run.start",
                json!({ "run_id": "lasting" }),
            )
            .await;
            starting.close().await;
            while starting.next().await.is_some() {}
            tokio::time::sleep(60 * MINUTE).await;
            let mut next = opened(&dialer, cli_hello()).await;
            let status = call(&mut next, 1, "daemon.status", json!({})).await;
            call(&mut next, 2, "run.finish", json!({ "run_id": "lasting" })).await;
            next.close().await;
            while next.next().await.is_some() {}
            tokio::time::sleep(11 * MINUTE).await;
            (
                status["result"]["live_runs"].clone(),
                answers(&dialer).await,
            )
        },
    );

    assert_eq!(
        (
            live,
            after,
            index_row(&env, "lasting")["lifecycle"].clone(),
            format!("{end:?}")
        ),
        (json!(1), false, json!("finished"), "Idle".to_owned())
    );
}

/// Covers: error:not_a_daemon
#[test]
fn an_attached_engine_answers_daemon_status_with_not_a_daemon() {
    let received = super::exchange(
        &engine(Product::Collie),
        &[
            super::hello("collie", "0.1"),
            super::request(1, "daemon.status"),
        ],
    );

    assert_eq!(received[1]["error"]["data"]["code"], json!("not_a_daemon"));
}

/// A Daemon that refuses a Client by Product hands over to its major's pick
/// only when it is not the pick and is idle; the refusal says so, and the
/// Client waits for the pick (ADR 0098).
///
/// Covers: story:3-5, story:3-138, error:not_in_product
#[rstest]
#[case::idle_and_not_the_pick(false, false, true)]
#[case::busy_and_not_the_pick(false, true, false)]
#[case::already_the_pick(true, false, false)]
fn a_daemon_refusing_a_product_hands_over_only_when_idle_and_not_the_pick(
    #[case] pick: bool,
    #[case] busy: bool,
    #[case] handover: bool,
) {
    assert_eq!(
        refusal(
            hello_with("foxhound", "0.1", "sdk_typescript", Some(TOKEN)),
            pick,
            busy
        ),
        (json!(handover), true, u32::from(handover))
    );
}

/// A Daemon that refuses a Client by version hands over to its major's pick
/// when it is not the pick and is idle; the refusal says so (ADR 0098).
///
/// Covers: story:3-5, story:3-138, error:protocol_version_unsupported
#[test]
fn a_daemon_refusing_a_version_hands_over_when_idle_and_not_the_pick() {
    assert_eq!(
        refusal(
            hello_with("collie", "0.2", "sdk_typescript", Some(TOKEN)),
            false,
            false
        ),
        (json!(true), true, 1)
    );
}

/// Sends `refused` to a Collie Daemon that is the pick or not, with another
/// Client connected when `busy`, and answers the refusal's `handover`,
/// whether the connection then closed, and how many handovers began.
fn refusal(refused: Value, pick: bool, busy: bool) -> (Value, bool, u32) {
    let handovers = no_handover();

    let counted = Arc::clone(&handovers);
    let (_, (answer, closed, begun)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        options(pick, &handovers),
        |dialer| async move {
            let other = if busy {
                Some(opened(&dialer, cli_hello()).await)
            } else {
                None
            };
            let mut client = connect(&dialer).await;
            client.send(&[refused]).await;
            let answer = client.next().await.unwrap();
            let closed = client.next().await.is_none();
            let begun = begun(&counted);
            if let Some(mut other) = other {
                other.close().await;
            }
            (answer["error"]["data"].clone(), closed, begun)
        },
    );

    (
        answer.get("handover").cloned().unwrap_or(json!(false)),
        closed,
        begun,
    )
}

/// A Daemon that refuses a `hello` keeps reading what the Client still
/// sends until the Client closes, so a call written behind the `hello`
/// after the refusal meets an open Connection, never a closed one: the
/// Client reads the refusal and then the end (ADR 0066).
///
/// Covers: story:3-132, error:protocol_version_unsupported
#[test]
fn a_call_written_after_a_refused_hello_meets_an_open_connection() {
    let (_, (code, ended, written)) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut client = connect(&dialer).await;
            client
                .send(&[hello_with("collie", "1.0", "sdk_typescript", Some(TOKEN))])
                .await;
            let refusal = client.next().await.unwrap();
            let ended = client.next().await.is_none();
            // On the paused clock, the sleep ends once the Daemon has done
            // all it does without the Client.
            tokio::time::sleep(Duration::from_millis(1)).await;
            let written = client.try_send(&[super::request(1, "engine.info")]).await;
            client.close().await;
            (refusal["error"]["data"]["code"].clone(), ended, written)
        },
    );

    assert_eq!(
        (code, ended, written.map_err(|error| error.kind())),
        (json!("protocol_version_unsupported"), true, Ok(()))
    );
}

/// A refused Client that never closes is closed all the same: the Daemon
/// stops reading what it sends.
///
/// Covers: story:3-132
#[test]
fn a_refused_client_that_never_closes_is_closed() {
    let (_, written) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        the_pick(),
        |dialer| async move {
            let mut client = connect(&dialer).await;
            client
                .send(&[hello_with("collie", "1.0", "sdk_typescript", Some(TOKEN))])
                .await;
            client.next().await.unwrap();
            tokio::time::sleep(MINUTE).await;
            client.try_send(&[super::request(1, "engine.info")]).await
        },
    );

    assert_eq!(
        written.map_err(|error| error.kind()),
        Err(std::io::ErrorKind::BrokenPipe)
    );
}

/// A Daemon that is not its major's pick hands over once no Client but a
/// Kennel that only observes is left and no Run is live (ADRs 0098 and
/// 0141).
///
/// Covers: story:3-138
#[test]
fn a_daemon_not_the_pick_hands_over_once_only_an_observing_kennel_is_left() {
    let handovers = no_handover();

    let (end, kennel_closed) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        options(false, &handovers),
        |dialer| async move {
            let mut kennel = opened(&dialer, kennel_hello(false)).await;
            let mut script = opened(&dialer, sdk_hello()).await;
            call(&mut script, 1, "run.start", json!({ "run_id": "busy" })).await;
            script.close().await;
            while script.next().await.is_some() {}
            kennel.next().await.is_none()
        },
    );

    assert_eq!(
        (
            kennel_closed,
            begun(&handovers),
            matches!(end, DaemonEnd::HandedOver(_))
        ),
        (true, 1, true)
    );
}

/// A Daemon that is not its major's pick hands over once only a Kennel is
/// left and no Run is live, since a Kennel only observes it (ADR 0141); a
/// Kennel whose `hello` asks to keep the Daemon alive holds the handover
/// off while it stays connected.
///
/// Covers: story:3-138
#[rstest]
#[case::observing(false, 1)]
#[case::keeping_it_alive(true, 0)]
fn a_daemon_not_the_pick_hands_over_with_only_a_kennel_left_unless_it_keeps_the_daemon_alive(
    #[case] keep_daemon_alive: bool,
    #[case] handed_over_while_connected: u32,
) {
    let handovers = no_handover();
    let counted = Arc::clone(&handovers);

    let (end, while_connected) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        options(false, &handovers),
        |dialer| async move {
            let kennel = opened(&dialer, kennel_hello(keep_daemon_alive)).await;
            let mut script = opened(&dialer, sdk_hello()).await;
            call(&mut script, 1, "run.start", json!({ "run_id": "busy" })).await;
            script.close().await;
            while script.next().await.is_some() {}
            tokio::time::sleep(MINUTE).await;
            let while_connected = begun(&counted);
            drop(kennel);
            while_connected
        },
    );

    assert_eq!(
        (while_connected, matches!(end, DaemonEnd::HandedOver(_))),
        (handed_over_while_connected, true)
    );
}

/// A Daemon that finds the Run index's lowest minor raised past itself
/// takes no new Run, and once its live Runs end hands over to its major's
/// pick though its Client stays connected, closing it; with the index
/// unraised its new Run starts, and it keeps its Client until the Client
/// leaves (ADR 0098).
///
/// Covers: story:3-144, story:3-138, error:format_version_unsupported
#[rstest]
#[case::raised(true, (Some("format_version_unsupported".to_owned()), true, 1))]
#[case::unraised(false, (None, false, 0))]
fn a_daemon_locked_out_of_the_run_index_hands_over_once_its_runs_end(
    #[case] raised: bool,
    #[case] expected: (Option<String>, bool, u32),
) {
    let env = TestEnv::new();
    let handovers = no_handover();
    let counted = Arc::clone(&handovers);
    let major = super::per_major(&env);

    let (end, observed) = daemon_within(
        &engine(Product::Collie),
        &env,
        BlockingPolicy::Inline,
        options(false, &handovers),
        MINUTE,
        |dialer| async move {
            let mut client = opened(&dialer, sdk_hello()).await;
            call(&mut client, 1, "run.start", json!({ "run_id": "busy" })).await;
            if raised {
                raise_the_run_index(&major, 5);
            }
            let late = call(&mut client, 2, "run.start", json!({ "run_id": "late" })).await;
            call(&mut client, 3, "run.finish", json!({ "run_id": "busy" })).await;
            let closed = closed_within_a_second(&mut client).await;
            let while_connected = begun(&counted);
            if !closed {
                call(&mut client, 4, "run.finish", json!({ "run_id": "late" })).await;
            }
            client.close().await;
            let refused = late["error"]["data"]["code"].as_str().map(str::to_owned);
            (refused, closed, while_connected)
        },
    );

    assert_eq!(
        (observed, matches!(end, Some(DaemonEnd::HandedOver(_)))),
        (expected, true)
    );
}

/// The pick that a Daemon locked out of the Run index handed over to
/// rebuilds, from its run state, the row of the Run that ended after the
/// lockout, which the index still listed as running (ADRs 0031 and 0098).
///
/// Covers: story:3-144
#[test]
fn the_pick_rebuilds_the_row_a_locked_out_daemon_could_not_write() {
    let env = TestEnv::new();
    let handovers = no_handover();
    let major = super::per_major(&env);

    let (locked_out, ()) = daemon_within(
        &engine(Product::Collie),
        &env,
        BlockingPolicy::Inline,
        options(false, &handovers),
        MINUTE,
        |dialer| async move {
            let mut client = opened(&dialer, sdk_hello()).await;
            call(&mut client, 1, "run.start", json!({ "run_id": "busy" })).await;
            raise_the_run_index(&major, 5);
            call(&mut client, 2, "run.finish", json!({ "run_id": "busy" })).await;
            closed_within_a_second(&mut client).await;
            client.close().await;
        },
    );
    let left = index_row(&env, "busy")["lifecycle"].clone();
    daemon_within(
        &super::engine_at(Product::Collie, "1.5.0"),
        &env,
        BlockingPolicy::Inline,
        options(true, &handovers),
        MINUTE,
        |dialer| async move {
            let mut client = opened(&dialer, sdk_hello()).await;
            // A Run call is answered once the Daemon's startup ended.
            call(&mut client, 1, "run.start", json!({ "run_id": "next" })).await;
            call(&mut client, 2, "run.finish", json!({ "run_id": "next" })).await;
            client.close().await;
        },
    );

    assert_eq!(
        (
            matches!(locked_out, Some(DaemonEnd::HandedOver(_))),
            left,
            index_row(&env, "busy")["lifecycle"].clone(),
        ),
        (true, json!("running"), json!("finished"))
    );
}

/// Whether the Daemon closes `client`'s Connection within a second: on
/// the paused clock, the wait ends at once when it does, else once it has
/// done all it does.
async fn closed_within_a_second(client: &mut Client) -> bool {
    tokio::time::timeout(Duration::from_secs(1), async {
        while client.next().await.is_some() {}
    })
    .await
    .is_ok()
}

/// Raises the lowest minor of the Run index in the per-major folder
/// `major` to `lowest_minor`, as a newer engine of the major would.
fn raise_the_run_index(major: &std::path::Path, lowest_minor: u16) {
    let index = rusqlite::Connection::open(major.join("run-index.sqlite")).unwrap();
    crate::lowest_minor::write_sqlite(
        &index,
        goodfetch_protocol::formats::RUN_INDEX.name,
        crate::lowest_minor::Marker {
            version: goodfetch_protocol::formats::RUN_INDEX.version,
            lowest_minor,
        },
    )
    .unwrap();
}

/// `daemon.stop` that arrives while a handover is decided, beside a
/// Client's end, never holds the Daemon: the handover goes on, the stop is
/// answered and changes nothing, and the Daemon ends handed over within a
/// bound.
///
/// Covers: story:3-138, operation:daemon.stop
#[test]
fn a_stop_asked_while_a_handover_is_decided_ends_the_daemon_handed_over() {
    let env = TestEnv::new();
    let handovers = no_handover();
    let counted = Arc::clone(&handovers);
    let (go, gate) = std::sync::mpsc::channel();
    #[expect(
        clippy::disallowed_types,
        reason = "the Gated succession's receiver, as on its field"
    )]
    let options = DaemonOptions {
        succession: Arc::new(Gated {
            begun: Arc::clone(&handovers),
            go: std::sync::Mutex::new(gate),
        }),
        ..the_pick()
    };

    let (end, (stopping, refusal)) = daemon_within(
        &engine(Product::Collie),
        &env,
        // The handover waits on a blocking thread, which holds the paused
        // clock still meanwhile.
        BlockingPolicy::Threads,
        options,
        MINUTE,
        |dialer| async move {
            let mut asking = opened(&dialer, kennel_hello(false)).await;
            let mut leaving = opened(&dialer, kennel_hello(false)).await;
            let mut refused = connect(&dialer).await;
            refused
                .send(&[hello_with("collie", "0.2", "sdk_typescript", Some(TOKEN))])
                .await;
            counted
                .subscribe()
                .wait_for(|count| *count == 1)
                .await
                .unwrap();
            leaving.close().await;
            while leaving.next().await.is_some() {}
            let stopping = call(&mut asking, 1, "daemon.stop", json!({})).await;
            go.send(()).unwrap();
            let refusal = refused.next().await.unwrap();
            refused.close().await;
            (
                stopping["result"].clone(),
                refusal["error"]["data"]["handover"].clone(),
            )
        },
    );

    assert_eq!(
        (
            end.map(|end| format!("{end:?}")),
            stopping,
            refusal,
            begun(&handovers)
        ),
        (
            Some("HandedOver".to_owned()),
            json!({ "live_runs": 0 }),
            json!(true),
            1
        )
    );
}

/// A Connection that ends while the Daemon drains, and so receives no end,
/// waits for room to report its own only until the Daemon's end closes it:
/// more such ends than the Daemon holds never keep it from ending at its
/// drain grace.
///
/// Covers: story:3-99
#[test]
fn connections_that_end_while_the_daemon_drains_never_keep_it_from_ending() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(stuck()));
    let asked = CancellationToken::new();

    let (end, closed) = daemon_within(
        &engine,
        &env,
        BlockingPolicy::Inline,
        draining(&asked),
        DRAIN_GRACE + MINUTE,
        |dialer| async move {
            let mut client = opened(&dialer, cli_hello()).await;
            let mut leaving = Vec::new();
            // More than the 16 ends the Daemon holds before a Connection
            // waits for room.
            for _ in 0..17 {
                leaving.push(opened(&dialer, sdk_hello()).await);
            }
            call(&mut client, 1, "test.stuck", json!({ "run_id": "stuck" })).await;
            asked.cancel();
            // The drain began, and waits for the stuck Run.
            tokio::time::sleep(MINUTE).await;
            for mut left in leaving {
                left.close().await;
                while left.next().await.is_some() {}
            }
            client.next().await.is_none()
        },
    );

    assert_eq!(
        (end.map(|end| format!("{end:?}")), closed),
        (Some("Drained".to_owned()), true)
    );
}

/// Starts a Run its driver ends, which reaches no safe point: its task
/// never ends, even cancelled, as one whose last write outlasts the drain
/// grace.
const STUCK: goodfetch_protocol::catalog::Operation = goodfetch_protocol::catalog::Operation {
    name: "test.stuck",
    description: "Starts a Run that reaches no safe point.",
    class: goodfetch_protocol::catalog::OperationClass::Effect,
    kind: goodfetch_protocol::catalog::OperationKind::RoundTrip,
    reach: goodfetch_protocol::catalog::Reach::AllSurfaces,
    params: goodfetch_protocol::catalog::schema_of::<Value>,
    result: goodfetch_protocol::catalog::schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: goodfetch_protocol::catalog::Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

pub(super) fn stuck() -> Contribution {
    Contribution::new("test-stuck").with_handler(stuck_handler())
}

/// The handler of `test.stuck`, for a contribution that carries other
/// calls too.
pub(super) fn stuck_handler() -> Handler {
    Handler::new(&STUCK, |params: Value, context: CallContext| async move {
        let run = driven(&context, &params).await;
        run.spawn(std::future::pending());
        Ok::<Value, CallError>(json!({ "run_id": run.id().to_string() }))
    })
}

/// The new Run of the Run ID in `params`, which its driver ends, begun
/// for a call in `context`.
pub(super) async fn driven(context: &CallContext, params: &Value) -> Run {
    let started = context
        .runs()
        .start(Start {
            run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
            tags: Tags::default(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Driver,
            initialize: None,
            policy_version: None,
            weakened: Vec::new(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: None,
            timeout: None,
            settings: None,
            detached: false,
        })
        .await
        .unwrap();
    let Started::New(run) = started else {
        panic!("the Run began before");
    };
    run
}

/// The pick, kept alive as `daemon run` is, draining once `asked` is
/// cancelled, with the default drain grace.
pub(super) fn draining(asked: &CancellationToken) -> DaemonOptions {
    DaemonOptions {
        keep_alive: true,
        drain: Drain {
            asked: Some(asked.clone()),
            grace: DRAIN_GRACE,
        },
        ..the_pick()
    }
}

/// `run.resume` of `run_id` on an Attached engine in `env`: how it
/// started.
fn resumed(engine: &Engine, env: &TestEnv, run_id: &str) -> Value {
    attached_on(engine, env, |mut client| async move {
        client.send(&[super::hello("collie", "0.1")]).await;
        let _ = client.next().await;
        let resumed = call(&mut client, 1, "run.resume", json!({ "run_id": run_id })).await;
        resumed["result"]["started_as"].clone()
    })
}

/// On `SIGTERM`, `daemon run` drains (ADR 0049): each live Run stops at its
/// next safe point with a Checkpoint, `stopped` for the drain and
/// resumable by its Run ID on the next start, and the Daemon ends.
///
/// Covers: story:3-99, operation:run.resume
#[test]
fn a_drained_daemon_stops_each_live_run_with_a_checkpoint_resumable_by_its_run_id() {
    let env = TestEnv::new();
    let engine = engine(Product::Collie);
    let asked = CancellationToken::new();

    let (end, closed) = daemon(&engine, &env, draining(&asked), |dialer| async move {
        let mut client = opened(&dialer, cli_hello()).await;
        call(&mut client, 1, "run.start", json!({ "run_id": "drained" })).await;
        asked.cancel();
        client.next().await.is_none()
    });

    let folder = run_folder(&env, "drained");
    let stopped = run_row(&folder);
    assert_eq!(
        (
            format!("{end:?}"),
            closed,
            stopped["lifecycle"].clone(),
            stopped["stop_reason"].clone(),
            folder.join("checkpoint.sqlite").is_file(),
            resumed(&engine, &env, "drained"),
        ),
        (
            "Drained".to_owned(),
            true,
            json!("stopped"),
            json!({ "kind": "drain" }),
            true,
            json!("resumed"),
        )
    );
}

/// A draining Daemon takes no run, starting or resuming, and ends once its
/// drain grace passes on its `Clock`; a Run that reached no safe point by
/// then stays `running`, resumable from its run file (ADR 0049), and the
/// Daemon logs how many it left so.
///
/// Covers: story:3-99, error:daemon_draining
#[test]
fn a_draining_daemon_refuses_runs_and_ends_at_its_grace_leaving_a_stuck_run_resumable() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(stuck()));
    let asked = CancellationToken::new();

    let mut ended = None;
    let records = records_of("goodfetch_engine::daemon", || {
        ended = Some(daemon(
            &engine,
            &env,
            draining(&asked),
            |dialer| async move {
                let mut client = opened(&dialer, cli_hello()).await;
                call(&mut client, 1, "test.stuck", json!({ "run_id": "stuck" })).await;
                asked.cancel();
                // The drain began, and waits for the stuck Run.
                tokio::time::sleep(MINUTE).await;
                let started = call(&mut client, 2, "run.start", json!({ "run_id": "late" })).await;
                let resumed =
                    call(&mut client, 3, "run.resume", json!({ "run_id": "stuck" })).await;
                while client.next().await.is_some() {}
                [started, resumed].map(|refused| refused["error"]["data"]["code"].clone())
            },
        ));
    });
    let (end, refusals) = ended.unwrap();

    assert_eq!(
        (
            format!("{end:?}"),
            refusals,
            run_row(&run_folder(&env, "stuck"))["lifecycle"].clone(),
            run_folder(&env, "late").exists(),
            resumed(&engine, &env, "stuck"),
            // Its warning at the grace, not its count when the drain began.
            records
                .iter()
                .any(|record| record.starts_with(": the drain grace passed")
                    && record.contains("live_runs=1")),
        ),
        (
            "Drained".to_owned(),
            [json!("daemon_draining"), json!("daemon_draining")],
            json!("running"),
            false,
            json!("resumed"),
            true,
        ),
        "{records:#?}"
    );
}
