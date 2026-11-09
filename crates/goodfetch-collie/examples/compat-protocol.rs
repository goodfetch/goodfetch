//! The JSON-RPC protocol's writer for the Compatibility corpus (ADR 0066):
//! runs a fixed scenario against Collie's engine over the in-memory network
//! and writes, into the folder given as the only argument, the messages the
//! Client sent as `client.jsonl` and those the engine answered as
//! `engine.jsonl`, one per line. Each call waits for its reply, so the
//! transcript is byte-stable. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime and writes files (ADR 0066)"
)]

use std::path::PathBuf;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_test_client::hello;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// What the Client sends; each request waits for its reply.
fn scenario() -> Vec<Value> {
    vec![
        hello(Product::Collie),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 1 } }),
        json!({ "jsonrpc": "2.0", "id": "echo", "method": "sample.echo", "params": { "text": "é" } }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "sample.crawl", "params": { "echo": { "text": "a" }, "pages": 1 } }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "page.levitate", "params": {} }),
    ]
}

fn lines(messages: &[Value]) -> Vec<u8> {
    let mut text = Vec::new();
    for message in messages {
        text.extend(serde_json::to_vec(message).expect("invariant: JSON values encode"));
        text.push(b'\n');
    }
    text
}

/// The scenario's engine runs on a home of its own, the test `Clock` and a
/// fixed `Entropy` seed, so its answers never vary (ADR 0066).
fn dependencies(home: &std::path::Path) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()),
        // 2026-01-05T09:00:00Z, test support's `START_TIME`.
        clock: Clock::paused(std::time::UNIX_EPOCH + std::time::Duration::from_hours(491_001)),
        entropy: Entropy::seeded(0),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: compat-protocol <folder>")?,
    );
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0")?,
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &goodfetch_collie::composition())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let sent = scenario();
    let home = tempfile::tempdir()?;
    let answered = runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await?.split();
            let mut replies = BufReader::new(reader).lines();
            let mut answered = Vec::new();
            for message in &sent {
                writer
                    .write_all(&lines(std::slice::from_ref(message)))
                    .await?;
                if message.get("id").is_some() {
                    let line = replies
                        .next_line()
                        .await?
                        .ok_or("the engine closed early")?;
                    answered.push(serde_json::from_str::<Value>(&line)?);
                }
            }
            writer.shutdown().await?;
            Ok::<_, Box<dyn std::error::Error>>(answered)
        };
        let (served, answered) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client
        );
        served?;
        answered
    })?;
    std::fs::write(folder.join("client.jsonl"), lines(&sent))?;
    std::fs::write(folder.join("engine.jsonl"), lines(&answered))?;
    Ok(())
}
