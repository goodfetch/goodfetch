//! The Run index's writer for the Compatibility corpus (ADR 0066): runs a
//! fixed scenario against Collie's engine over the in-memory network, on the
//! test `Clock` and a fixed `Entropy` seed, in a home of its own (a Run that
//! finishes and one that is aborted, both tagged), and writes the Run index
//! it leaves, as SQL text, into the folder given as the only argument, as
//! `run-index.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that builds its runtime, writes files and runs on the test Clock (ADRs 0059 and 0066)"
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

/// What the Client asks, in order; each call waits for its reply.
fn scenario() -> Vec<Value> {
    let call = |id: i64, method: &str, params: Value| json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    vec![
        call(
            0,
            "hello",
            json!({ "protocol_version": "0.1", "surface": "rust", "needs": "collie", "product_version": "0.0.0" }),
        ),
        call(
            1,
            "run.start",
            json!({ "run_id": "finished-run", "tags": ["schedule:nightly", "team:shop"] }),
        ),
        call(2, "run.finish", json!({ "run_id": "finished-run" })),
        call(
            3,
            "run.start",
            json!({ "run_id": "aborted-run", "tags": ["manual"] }),
        ),
        call(4, "run.abort", json!({ "run_id": "aborted-run" })),
    ]
}

fn dependencies(home: &Path) -> Dependencies {
    Dependencies {
        home: Home::new(home.to_owned()),
        // 2026-01-05T09:00:00Z, test support's `START_TIME`.
        clock: Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001)),
        entropy: Entropy::seeded(0),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: compat-run-index <folder>")?,
    );
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0")?,
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &goodfetch_collie::composition())?;
    let home = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()?;
    runtime.block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer.connect().await?.split();
            let mut replies = BufReader::new(reader).lines();
            for message in scenario() {
                let mut line = serde_json::to_vec(&message)?;
                line.push(b'\n');
                writer.write_all(&line).await?;
                let reply: Value = serde_json::from_str(
                    &replies
                        .next_line()
                        .await?
                        .ok_or("the engine closed early")?,
                )?;
                if reply.get("error").is_some() {
                    return Err(format!("the scenario failed: {reply}").into());
                }
            }
            writer.shutdown().await?;
            while replies.next_line().await?.is_some() {}
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let (served, talked) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client
        );
        served?;
        talked
    })?;
    let index = rusqlite::Connection::open(home.path().join("0.0").join("run-index.sqlite"))?;
    std::fs::write(
        folder.join("run-index.sql"),
        goodfetch_test_support::sql_dump(&index)?,
    )?;
    Ok(())
}
