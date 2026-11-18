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
use goodfetch_collie_client::{Connection, ConnectionRuntime, Error};
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
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};
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
fn engine() -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: version(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default().with(goodfetch_engine::contribution());
    Engine::new(identity, &composition).unwrap()
}

/// What the engine runs on, its clock paused at the epoch.
fn dependencies(home: &TempDir) -> Dependencies {
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
            let (connection, said) = Connection::open(
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
        assert!(matches!(after, Err(Error::Closed)), "{after:?}");
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
            let (connection, _) = Connection::open(
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

#[test]
fn a_call_past_its_timeout_fails_and_a_late_reply_to_it_changes_nothing() {
    use tokio::io::AsyncWriteExt as _;
    let (network, mut lines, mut writer) = scripted();
    let said = serde_json::json!({
        "protocol_version": "0.1",
        "product": "collie",
        "product_version": "0.0.0",
        "engine_instance": "01J0000000000000000000000",
    });
    let info = serde_json::json!({"product": "collie", "product_version": "0.0.0", "protocol_version": "0.1", "product_major": 0});
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
            let (connection, _) = Connection::open(
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

        assert!(matches!(expired, Err(Error::TimedOut)), "{expired:?}");
        assert_eq!(answered.unwrap().product, Product::Collie);
    });
}
