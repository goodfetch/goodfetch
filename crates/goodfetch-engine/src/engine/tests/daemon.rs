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
use super::{Client, attached_on, dependencies, engine, engine_with};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::daemon::{DaemonEnd, DaemonOptions, Drain, Holds, Succession};
use crate::engine::Engine;
use crate::ops::Ops;
use crate::runs::{EndedBy, Start, Started};
use crate::transport::{self, InMemoryDialer, Network};

mod listener;

pub(super) const TOKEN: &str = "the-launch-token";

const MINUTE: Duration = Duration::from_secs(60);

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
        drain: None,
        install: None,
        ops: Ops::default(),
        websocket: None,
    }
}

/// The pick's options.
pub(super) fn the_pick() -> DaemonOptions {
    options(true, &no_handover())
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
    goodfetch_test_support::runtime().block_on(async {
        let dependencies = dependencies(env);
        let clock = dependencies.clock;
        let unset = tracing::dispatcher::get_default(
            tracing::Dispatch::is::<tracing::subscriber::NoSubscriber>,
        );
        let _subscriber = unset.then(|| {
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }))
        });
        let (network, dialer) = transport::in_memory();
        let (listener, _) = Network::InMemory(network)
            .listen(std::path::Path::new("daemon.sock"))
            .await
            .unwrap();
        tokio::join!(
            engine.run_daemon(listener, dependencies, options),
            body(dialer)
        )
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
fn sdk_hello() -> Value {
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

    assert_eq!(
        (answer, named),
        (
            json!({
                "jsonrpc": "2.0", "id": 0,
                "result": { "protocol_version": "0.1", "product": "collie", "product_version": "1.4.0" },
            }),
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
    stability: goodfetch_protocol::catalog::Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

pub(super) fn stuck() -> Contribution {
    Contribution::new("test-stuck").with_handler(Handler::new(
        &STUCK,
        |params: Value, context: CallContext| async move {
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
                })
                .await
                .unwrap();
            let Started::New(run) = started else {
                panic!("the Run began before");
            };
            run.spawn(std::future::pending());
            Ok::<Value, CallError>(json!({ "run_id": run.id().to_string() }))
        },
    ))
}

/// The pick, kept alive as `daemon run` is, draining once `asked` is
/// cancelled, with the default drain grace.
pub(super) fn draining(asked: &CancellationToken) -> DaemonOptions {
    DaemonOptions {
        keep_alive: true,
        drain: Some(Drain {
            asked: asked.clone(),
            grace: Duration::from_secs(110),
        }),
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
/// then stays `running`, resumable from its run file (ADR 0049).
///
/// Covers: story:3-99, error:daemon_draining
#[test]
fn a_draining_daemon_refuses_runs_and_ends_at_its_grace_leaving_a_stuck_run_resumable() {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(stuck()));
    let asked = CancellationToken::new();

    let (end, refusals) = daemon(&engine, &env, draining(&asked), |dialer| async move {
        let mut client = opened(&dialer, cli_hello()).await;
        call(&mut client, 1, "test.stuck", json!({ "run_id": "stuck" })).await;
        asked.cancel();
        // The drain began, and waits for the stuck Run.
        tokio::time::sleep(MINUTE).await;
        let started = call(&mut client, 2, "run.start", json!({ "run_id": "late" })).await;
        let resumed = call(&mut client, 3, "run.resume", json!({ "run_id": "stuck" })).await;
        while client.next().await.is_some() {}
        [started, resumed].map(|refused| refused["error"]["data"]["code"].clone())
    });

    assert_eq!(
        (
            format!("{end:?}"),
            refusals,
            run_row(&run_folder(&env, "stuck"))["lifecycle"].clone(),
            run_folder(&env, "late").exists(),
            resumed(&engine, &env, "stuck"),
        ),
        (
            "Drained".to_owned(),
            [json!("daemon_draining"), json!("daemon_draining")],
            json!("running"),
            false,
            json!("resumed"),
        )
    );
}
