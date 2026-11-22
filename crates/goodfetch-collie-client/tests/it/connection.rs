//! A Connection over the in-memory network (spec #606, ADR 0081): it says
//! `hello`, makes a call and closes, on the test helper's paused
//! current-thread runtime or on a runtime it owns, which dropping its last
//! handle shuts down in the background even on an async worker.
//! Covers: story:11-60

use std::future::Future;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::time::Duration;

use goodfetch_collie_client::transport::{Dial, Endpoint, Link, Network};
use goodfetch_collie_client::{
    ClientConnection, ConnectionRuntime, Error, ErrorCode, ListenerQueue,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::Keychain;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, InMemoryDialer, InMemoryNetwork};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{
    DeadlineExceededDetails, HelloParams, ProductVersion, Surface,
};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};
use goodfetch_protocol::page::PAGE_GOTO;
use goodfetch_protocol::settings::SettingInvalidDetails;
use tempfile::TempDir;

/// The Product version both ends speak.
fn version() -> ProductVersion {
    ProductVersion::parse("0.0.0").unwrap()
}

/// A Rust program's `hello`, needing Collie.
pub(crate) fn hello() -> HelloParams {
    HelloParams {
        needs: Some(Product::Collie),
        ..HelloParams::new(Surface::Rust, version())
    }
}

/// A Collie engine with only the engine's own operations, on a Goodfetch
/// home in `home`.
pub(crate) fn engine() -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: version(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default().with(goodfetch_engine::contribution());
    Engine::new(identity, &composition).unwrap()
}

/// What the engine runs on, its clock paused at the epoch.
pub(crate) fn dependencies(home: &TempDir) -> Dependencies {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Clock's and the Entropy's test versions, in a test (ADR 0059)"
    )]
    let (clock, entropy) = (Clock::paused(std::time::UNIX_EPOCH), Entropy::seeded(0));
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: Keychain::absent(),
        home: Home::new(home.path().to_owned()),
        clock,
        entropy,
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: PolitenessOptions::default(),
    }
}

/// The engine's in-memory dialer as the client crate dials.
struct InMemory(InMemoryDialer);

impl Dial for InMemory {
    fn dial(&self) -> Pin<Box<dyn Future<Output = std::io::Result<Link>> + Send + '_>> {
        Box::pin(async {
            let (reader, writer) = self
                .0
                .connect()
                .await
                .map_err(std::io::Error::other)?
                .split();
            Ok(Link::new(reader, writer))
        })
    }
}

/// An in-memory network: the engine's end and the client's.
fn in_memory() -> (InMemoryNetwork, Network) {
    let (network, dialer) = transport::in_memory();
    (network, Network::in_memory(InMemory(dialer)))
}

/// Serves the engine over `network` until its Client closes.
async fn serve(network: InMemoryNetwork, home: &TempDir) {
    engine()
        .run_attached(transport::Network::InMemory(network), dependencies(home))
        .await
        .unwrap();
}

#[test]
fn a_connection_says_hello_calls_and_closes_on_the_paused_runtime() {
    let home = tempfile::tempdir().unwrap();
    let (engine_end, network) = in_memory();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let client = async {
            let (connection, said) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let info: EngineInfo = connection
                .call(ENGINE_INFO.name, &EngineInfoParams::default())
                .await
                .unwrap();
            connection.close().await.unwrap();
            (said, info, connection)
        };
        let ((said, info, connection), ()) = tokio::join!(client, serve(engine_end, &home));

        assert_eq!(said.product, Product::Collie);
        assert_eq!(info.product_version, version());
        let after: Result<EngineInfo, Error> = connection
            .call(ENGINE_INFO.name, &EngineInfoParams::default())
            .await;
        assert!(
            matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
            "{after:?}"
        );
    });
}

#[test]
fn closing_and_dropping_on_an_async_worker_shuts_the_owned_runtime_down_in_the_background() {
    let home = tempfile::tempdir().unwrap();
    let (engine_end, network) = in_memory();
    let one = NonZeroUsize::MIN;
    let owned = goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::owned(owned),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            connection.close().await.unwrap();
            drop(connection);
        };
        tokio::join!(client, serve(engine_end, &home));
    });
}

/// A peer the test scripts, at the end of an in-memory pipe.
struct Scripted(tokio::sync::Mutex<Option<Link>>);

impl Dial for Scripted {
    fn dial(&self) -> Pin<Box<dyn Future<Output = std::io::Result<Link>> + Send + '_>> {
        let link = self.0.try_lock().unwrap().take();
        Box::pin(async { link.ok_or_else(|| std::io::Error::other("dialed twice")) })
    }
}

/// A network whose one Connection reaches the lines the test reads and
/// writes.
pub(crate) fn scripted() -> (
    Network,
    tokio::io::Lines<tokio::io::BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    tokio::io::WriteHalf<tokio::io::DuplexStream>,
) {
    use tokio::io::AsyncBufReadExt as _;
    let (client, peer) = tokio::io::duplex(64 * 1024);
    let (client_reader, client_writer) = tokio::io::split(client);
    let (peer_reader, peer_writer) = tokio::io::split(peer);
    let network = Network::in_memory(Scripted(tokio::sync::Mutex::new(Some(Link::new(
        client_reader,
        client_writer,
    )))));
    (
        network,
        tokio::io::BufReader::new(peer_reader).lines(),
        peer_writer,
    )
}

/// The reply to the request `line` with `result`.
pub(crate) fn reply(line: &str, result: &serde_json::Value) -> Vec<u8> {
    let request: serde_json::Value = serde_json::from_str(line).unwrap();
    let mut reply = serde_json::to_vec(
        &serde_json::json!({"jsonrpc": "2.0", "id": request["id"], "result": result}),
    )
    .unwrap();
    reply.push(b'\n');
    reply
}

/// What a scripted peer's engine answers `hello` with.
pub(crate) fn said() -> serde_json::Value {
    serde_json::json!({
        "protocol_version": "0.1",
        "product": "collie",
        "product_version": "0.0.0",
        "engine_instance": "01J0000000000000000000000",
    })
}

/// What a scripted peer's engine answers `engine.info` with.
fn info() -> serde_json::Value {
    serde_json::json!({"product": "collie", "product_version": "0.0.0", "protocol_version": "0.1", "product_major": 0})
}

/// A call's `timeout` left out is not sent, `no_timeout` sends null, and a
/// duration its milliseconds rounded up (ADR 0080): the wire side of the
/// `call-timeout-three-states` case.
///
/// Covers: story:11-40, story:11-42
#[test]
fn a_call_sends_its_timeout_left_out_null_or_in_milliseconds_rounded_up() {
    use tokio::io::AsyncWriteExt as _;
    let (network, mut lines, mut writer) = scripted();
    let said = said();
    let started = serde_json::json!({"run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc", "lifecycle": "running", "needs": "collie", "started_as": "new"});
    let report = serde_json::json!({
        "run_id": "01ke6p4ym0j8x3v5wq2rtbn7hc",
        "lifecycle": "finished",
        "verdict": "verified",
        "completeness": "complete",
        "started_at": "2026-01-01T00:00:00Z",
        "ended_at": "2026-01-01T00:00:00Z",
        "usage": {
            "model": {"calls": 0, "tokens": 0, "input_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0, "output_tokens": 0, "reasoning_tokens": 0, "price": null},
            "browser_time": 0,
            "proxy_bytes": 0,
            "requests": 0,
        },
    });
    let runtime = goodfetch_test_support::runtime();
    let sent = runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said)).await.unwrap();
            let start = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&start, &started)).await.unwrap();
            // Every later request, the four finishes and what closing
            // sends, is answered with the Run's report.
            let mut sent = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                writer.write_all(&reply(&line, &report)).await.unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                if request["method"] == "run.finish" {
                    sent.push(request["params"].get("timeout").cloned());
                }
            }
            writer.shutdown().await.unwrap();
            sent
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let run = connection.start().await.unwrap();
            run.finish().await.unwrap();
            run.finish().no_timeout().await.unwrap();
            run.finish().timeout(Duration::from_secs(30)).await.unwrap();
            run.finish()
                .timeout(Duration::from_micros(1500))
                .await
                .unwrap();
            connection.close().await.unwrap();
        };
        let ((), sent) = tokio::join!(client, peer);
        sent
    });

    assert_eq!(
        sent,
        [
            None,
            Some(serde_json::Value::Null),
            Some(serde_json::json!(30_000)),
            Some(serde_json::json!(2)),
        ]
    );
}

#[test]
fn a_call_past_its_timeout_fails_and_a_late_reply_to_it_changes_nothing() {
    use tokio::io::AsyncWriteExt as _;
    let (network, mut lines, mut writer) = scripted();
    let said = said();
    let info = info();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said)).await.unwrap();
            let unanswered = lines.next_line().await.unwrap().unwrap();
            let answered = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&unanswered, &info)).await.unwrap();
            writer.write_all(&reply(&answered, &info)).await.unwrap();
            assert!(
                lines.next_line().await.unwrap().is_none(),
                "closing ends the input"
            );
            writer.shutdown().await.unwrap();
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let expired: Result<EngineInfo, Error> = connection
                .call(ENGINE_INFO.name, &EngineInfoParams::default())
                .timeout(Duration::from_secs(1))
                .await;
            let answered: Result<EngineInfo, Error> = connection
                .call(ENGINE_INFO.name, &EngineInfoParams::default())
                .no_timeout()
                .await;
            connection.close().await.unwrap();
            (expired, answered)
        };
        let ((expired, answered), ()) = tokio::join!(client, peer);

        assert!(
            matches!(&expired, Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED),
            "{expired:?}"
        );
        assert_eq!(answered.unwrap().product, Product::Collie);
    });
}

/// Dropping the future of a call the catalog says is cancellable, once the
/// call is sent, cancels it with `$/cancel` naming its id, without waiting
/// (ADR 0080); the peer's late reply to it and the next call then go as
/// usual: the program's own cancellation is never an error.
///
/// Covers: story:11-48
#[test]
fn a_dropped_cancellable_call_is_cancelled_by_its_id() {
    let read = after_an_ended_call(PAGE_GOTO.name, Cut::Dropped);

    assert_cancelled_then_next(&read);
}

/// A cancellable call whose timer, the client's own, runs out fails with
/// `deadline_exceeded` and is cancelled with `$/cancel` naming its id, so
/// the engine's work ends too (ADR 0080); the peer's late reply to it and
/// the next call then go as usual.
///
/// Covers: story:11-47
#[test]
fn a_cancellable_call_past_its_clients_timer_is_cancelled_by_its_id() {
    let read = after_an_ended_call(PAGE_GOTO.name, Cut::TimedOut);

    assert_cancelled_then_next(&read);
}

/// The peer read the call, then its `$/cancel` naming its id, then the next
/// call, and nothing else.
fn assert_cancelled_then_next(read: &[serde_json::Value]) {
    assert_eq!(read[0]["method"], PAGE_GOTO.name);
    assert_eq!(
        read[1],
        serde_json::json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": read[0]["id"]}}),
    );
    assert_eq!(read[2]["method"], ENGINE_INFO.name);
    assert_eq!(read.len(), 3);
}

/// Dropping the future of a call the catalog says is not cancellable sends
/// no `$/cancel`, which the engine would ignore (ADR 0080): the peer reads
/// the next call straight after it.
#[test]
fn a_dropped_call_that_is_not_cancellable_sends_no_cancel() {
    let read = after_an_ended_call(ENGINE_INFO.name, Cut::Dropped);

    let methods: Vec<_> = read.iter().map(|line| line["method"].clone()).collect();
    assert_eq!(methods, [ENGINE_INFO.name, ENGINE_INFO.name]);
}

/// How the client ends a call the peer leaves unanswered.
#[derive(Clone, Copy)]
enum Cut {
    /// The program drops its future once the peer has read it.
    Dropped,
    /// Its timer, the client's own, runs out; the call fails with
    /// `deadline_exceeded`.
    TimedOut,
}

/// Every line a peer reads after `hello` when the client ends a call of
/// `method`, as `cut` says, once the peer has read it, then makes an
/// `engine.info` call and closes; the peer answers the ended call only
/// after reading that next call.
fn after_an_ended_call(method: &'static str, cut: Cut) -> Vec<serde_json::Value> {
    use tokio::io::AsyncWriteExt as _;
    let (network, mut lines, mut writer) = scripted();
    let said = said();
    let info = info();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let (sent, heard) = tokio::sync::oneshot::channel::<()>();
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said)).await.unwrap();
            let dropped = lines.next_line().await.unwrap().unwrap();
            sent.send(()).unwrap();
            let mut read: Vec<serde_json::Value> = vec![serde_json::from_str(&dropped).unwrap()];
            while let Some(line) = lines.next_line().await.unwrap() {
                let line: serde_json::Value = serde_json::from_str(&line).unwrap();
                if line.get("id").is_some() {
                    writer.write_all(&reply(&dropped, &info)).await.unwrap();
                    writer
                        .write_all(&reply(&line.to_string(), &info))
                        .await
                        .unwrap();
                }
                read.push(line);
            }
            writer.shutdown().await.unwrap();
            read
        };
        let client = async {
            let (connection, _) = ClientConnection::open(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
            )
            .await
            .unwrap();
            let call = connection.call::<serde_json::Value>(method, &serde_json::json!({}));
            match cut {
                Cut::Dropped => tokio::select! {
                    replied = call.no_timeout() => panic!("the peer never answers before the drop: {replied:?}"),
                    _ = heard => {}
                },
                Cut::TimedOut => {
                    let expired = call.timeout(Duration::from_secs(1)).await;
                    assert!(
                        matches!(&expired, Err(error) if error.code() == ErrorCode::DEADLINE_EXCEEDED),
                        "{expired:?}"
                    );
                    heard.await.unwrap();
                }
            }
            let answered: Result<EngineInfo, Error> = connection
                .call(ENGINE_INFO.name, &EngineInfoParams::default())
                .no_timeout()
                .await;
            connection.close().await.unwrap();
            answered
        };
        let (answered, read) = tokio::join!(client, peer);

        assert_eq!(answered.unwrap().product, Product::Collie);
        read
    })
}

/// Opens a Connection to a peer that reads `hello` and answers it after
/// `answer_after`, or never, within `timeout`, on the paused clock; the
/// opening's result, and every request the peer read after `hello`.
fn open_within_against(
    timeout: goodfetch_collie_client::ConnectTimeout,
    answer_after: Option<Duration>,
) -> (Result<(), Error>, Vec<serde_json::Value>) {
    use tokio::io::AsyncWriteExt as _;
    let (network, mut lines, mut writer) = scripted();
    let said = said();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            if let Some(after) = answer_after {
                tokio::time::sleep(after).await;
                writer.write_all(&reply(&hello, &said)).await.unwrap();
            }
            let mut read = Vec::new();
            while let Some(line) = lines.next_line().await.unwrap() {
                read.push(serde_json::from_str(&line).unwrap());
            }
            writer.shutdown().await.unwrap();
            read
        };
        let client = async {
            let opened = ClientConnection::open_within(
                ConnectionRuntime::driven(tokio::runtime::Handle::current()),
                &network,
                &Endpoint::InMemory,
                hello(),
                timeout,
                ListenerQueue::DEFAULT,
                None,
                None,
            )
            .await;
            match opened {
                Ok((connection, _)) => connection.close().await,
                Err(error) => Err(error),
            }
        };
        tokio::join!(client, peer)
    })
}

/// The `deadline_exceeded` details of a failed opening.
fn deadline_details(opened: Result<(), Error>) -> serde_json::Value {
    let Err(error) = opened else {
        panic!("the opening should fail with deadline_exceeded: {opened:?}");
    };
    assert_eq!(error.code(), ErrorCode::DEADLINE_EXCEEDED, "{error:?}");
    serde_json::to_value(error.details::<DeadlineExceededDetails>().unwrap()).unwrap()
}

/// The Client conformance case `connect-timeout-three-states`, at the
/// client: a `hello` the engine never answers fails the opening with
/// `deadline_exceeded` once the connection's own `timeout` passes, naming
/// `timeout.connect` at the run layer, given as an argument, and closes
/// the Connection; left out, `timeout.connect`'s 30 s default ends it; with
/// no timeout, an answer a minute late still opens it.
/// Covers: setting:timeout.connect
#[test]
fn an_unanswered_hello_fails_the_opening_past_its_timeout_in_its_three_states() {
    use goodfetch_collie_client::ConnectTimeout;

    let (opened, read) = open_within_against(ConnectTimeout::After(Duration::from_secs(5)), None);
    let details = deadline_details(opened);
    assert_eq!(
        details["limit"],
        serde_json::json!({"key": "timeout.connect", "value": 5000, "layer": "run", "origin": "argument"})
    );
    assert_eq!(details["waited_for"], "hello");
    assert_eq!(details["elapsed"], 5000);
    assert!(
        read.iter().all(|request| request["method"] != "hello"),
        "nothing but closing follows `hello`: {read:?}"
    );

    let (opened, _) = open_within_against(ConnectTimeout::Default, None);
    let details = deadline_details(opened);
    assert_eq!(
        details["limit"],
        serde_json::json!({"key": "timeout.connect", "value": 30_000, "layer": "default"})
    );

    let (opened, _) = open_within_against(ConnectTimeout::Off, Some(Duration::from_secs(60)));
    opened.unwrap();
}

/// A connection's own `timeout` of zero is refused before anything is
/// sent, with `setting_invalid` naming the form with no timeout.
/// Covers: setting:timeout.connect
#[test]
fn a_zero_connect_timeout_is_refused_before_hello() {
    let refused = goodfetch_collie_client::ConnectTimeout::After(Duration::ZERO).check();
    let Err(error) = refused else {
        panic!("a zero timeout should be refused: {refused:?}");
    };
    assert_eq!(error.code(), ErrorCode::SETTING_INVALID);
    let details: SettingInvalidDetails = error.details().unwrap();
    assert_eq!(details.key, "timeout.connect");
    assert_eq!(
        details.fix.as_deref(),
        Some("give at least 1 ms, or `.no_timeout()` for no timeout")
    );
}
