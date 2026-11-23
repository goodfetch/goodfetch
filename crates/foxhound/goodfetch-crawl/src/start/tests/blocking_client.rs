//! The Client of the model-based tests, which drive an Attached engine
//! from outside its runtime, one message at a time, and stop it, crash it
//! and start another in the same home.

use goodfetch_engine::Dependencies;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::runtime::Runtime;

use super::engine;

/// The Client's end of an Attached engine's Connection, and the runtime the
/// engine runs on as a task of its own owner.
pub(super) struct BlockingClient {
    pub(super) runtime: Runtime,
    owner: TaskOwner,
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl BlockingClient {
    /// Runs an engine on `runtime`, on the `Dependencies` that
    /// `dependencies` makes there, and says `hello` to it.
    pub(super) fn attach(runtime: Runtime, dependencies: impl FnOnce() -> Dependencies) -> Self {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (reader, writer) = runtime.block_on(async { attach(&owner, dependencies()).await });
        let mut client = Self {
            runtime,
            owner,
            lines: BufReader::new(reader).lines(),
            writer,
        };
        client.hello();
        client
    }

    /// Runs another engine, once the last one stopped, and says `hello` to
    /// it.
    pub(super) fn reattach(&mut self, dependencies: impl FnOnce() -> Dependencies) {
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
        let owner = &self.owner;
        let (reader, writer) = self
            .runtime
            .block_on(async { attach(owner, dependencies()).await });
        self.lines = BufReader::new(reader).lines();
        self.writer = writer;
        self.hello();
    }

    pub(super) fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.runtime.block_on(self.writer.write_all(&line)).unwrap();
    }

    pub(super) fn next(&mut self) -> Value {
        let line = self
            .runtime
            .block_on(self.lines.next_line())
            .unwrap()
            .expect("the engine closed early");
        serde_json::from_str(&line).unwrap()
    }

    fn hello(&mut self) {
        self.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": { "protocol_version": "0.1", "surface": "rust", "needs": "foxhound", "product_version": "1.4.0" },
        }));
        assert_eq!(self.next()["result"]["product"], "foxhound");
    }

    /// Closes the Connection and waits for the engine to stop.
    pub(super) fn stop(&mut self) {
        self.runtime.block_on(self.writer.shutdown()).unwrap();
        while self
            .runtime
            .block_on(self.lines.next_line())
            .unwrap()
            .is_some()
        {}
        self.runtime.block_on(self.owner.wait());
    }
}

/// Runs an Attached engine on `dependencies` as a task of `owner`, and
/// connects to it.
async fn attach(
    owner: &TaskOwner,
    dependencies: Dependencies,
) -> (ConnectionReader, ConnectionWriter) {
    let (network, dialer) = transport::in_memory();
    let engine = engine();
    owner.spawn(async move {
        engine
            .run_attached(Network::InMemory(network), dependencies)
            .await
            .unwrap();
    });
    dialer.connect().await.unwrap().split()
}
