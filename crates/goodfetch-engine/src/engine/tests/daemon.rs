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

use super::run::{call, index_row};
use super::{Client, dependencies, engine};
use crate::daemon::{DaemonConsole, DaemonEnd, DaemonOptions, Holds, Succession};
use crate::engine::Engine;
use crate::transport::{self, InMemoryDialer, Network};

const TOKEN: &str = "the-launch-token";

const MINUTE: Duration = Duration::from_secs(60);

/// A Succession whose answer the test fixes, counting the handovers begun.
struct Fixed {
    pick: bool,
    handovers: Handovers,
}

/// How many handovers began.
type Handovers = Arc<watch::Sender<u32>>;

fn no_handover() -> Handovers {
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

fn options(pick: bool, handovers: &Handovers) -> DaemonOptions {
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
        console: DaemonConsole::Absent,
    }
}

/// The pick's options.
fn the_pick() -> DaemonOptions {
    options(true, &no_handover())
}

/// Runs `engine` as a Daemon with `options` in `env` while `body` dials it,
/// on the tests' paused runtime, and answers how it ended and what `body`
/// answered. As in `attached_with`, the test `tracing` subscriber writes the
/// dispatch records into the test's exercise record (ADR 0072).
fn daemon<F, Fut, T>(
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

async fn connect(dialer: &InMemoryDialer) -> Client {
    let (reader, writer) = dialer.connect().await.unwrap().split();
    Client {
        lines: BufReader::new(reader).lines(),
        writer,
    }
}

fn hello_with(product: &str, protocol_version: &str, surface: &str, token: Option<&str>) -> Value {
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

fn cli_hello() -> Value {
    hello_with("collie", "0.1", "cli", Some(TOKEN))
}

/// A Client that said `hello` with the token.
async fn opened(dialer: &InMemoryDialer, hello: Value) -> Client {
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

    assert_eq!(
        answer,
        Some(json!({
            "jsonrpc": "2.0", "id": 0,
            "result": { "protocol_version": "0.1", "product": "collie", "product_version": "1.4.0" },
        }))
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

/// A Client keeps the Daemon past its idle timeout while it is connected,
/// a visible Console tab as much as a script (ADR 0024).
///
/// Covers: story:3-18, story:3-19
#[rstest]
#[case::the_cli("cli")]
#[case::a_console_tab("console")]
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
            let mut second =
                opened(&dialer, hello_with("collie", "0.1", "console", Some(TOKEN))).await;
            call(&mut first, 1, "run.start", json!({ "run_id": "one" })).await;
            call(&mut second, 1, "run.start", json!({ "run_id": "two" })).await;
            let status = call(&mut first, 2, "daemon.status", json!({})).await;
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
            "console": "absent",
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
            let mut leaving = opened(&dialer, cli_hello()).await;
            let mut staying = opened(&dialer, cli_hello()).await;
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

/// A Daemon that is not its major's pick hands over once no Client but a
/// Console is left and no Run is live (ADR 0098).
///
/// Covers: story:3-138
#[test]
fn a_daemon_not_the_pick_hands_over_once_only_a_console_is_left() {
    let handovers = no_handover();

    let (end, console_closed) = daemon(
        &engine(Product::Collie),
        &TestEnv::new(),
        options(false, &handovers),
        |dialer| async move {
            let mut console =
                opened(&dialer, hello_with("collie", "0.1", "console", Some(TOKEN))).await;
            let mut script = opened(&dialer, cli_hello()).await;
            call(&mut script, 1, "run.start", json!({ "run_id": "busy" })).await;
            script.close().await;
            while script.next().await.is_some() {}
            console.next().await.is_none()
        },
    );

    assert_eq!(
        (
            console_closed,
            begun(&handovers),
            matches!(end, DaemonEnd::HandedOver(_))
        ),
        (true, 1, true)
    );
}
