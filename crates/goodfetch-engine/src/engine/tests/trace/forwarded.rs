//! A read this engine forwards to the engine that owns its Run (ADR 0102)
//! carries the Trace context of its own `CLIENT` span, the child of the
//! request it serves, as every request the engine sends does (ADR 0132).
//! The read of a raw call keeps its raw mark on the way (ADR 0099). The
//! owner is a stand-in on a Unix socket in the test's folder, which answers
//! `hello` and the read, and keeps the read's line.
#![expect(
    clippy::disallowed_methods,
    reason = "the owner listens on a real socket, so the test runs on a runtime whose clock is not paused (TESTING.md, Time), and writes the owner's token file"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{PrivateEndpoint, ProductVersion};
use goodfetch_protocol::run::RunOwner;
use goodfetch_test_support::TestEnv;
use opentelemetry::trace::SpanKind;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tracing::Instrument as _;

use super::{exported, the_span};
use crate::clock::Clock;
use crate::forward::ToOwner;
use crate::home::Home;
use crate::runs::Forwarder as _;
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};
use crate::transport::Network;

/// The span of the request the forwarding engine serves.
const SERVING: &str = "the request the engine serves";

/// Forwards `run.progress` to a stand-in owner under a span of its own,
/// for a raw call when `raw`, and answers the read's line as the owner
/// received it.
fn forwarded_read(env: &TestEnv, raw: bool) -> Value {
    let runtime = goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (mut listener, address) = Network::Os
            .listen(&env.folder().join("owner.sock"))
            .unwrap();
        let token_file = env.folder().join("owner.token");
        std::fs::write(&token_file, "the-owners-token").unwrap();
        let owner = RunOwner::Attached(PrivateEndpoint {
            address,
            token_file,
        });
        let home = Home::new(env.home().to_owned());
        let version = ProductVersion::parse("0.0.0").unwrap();
        let tasks = TaskOwner::root(BlockingPolicy::Inline);
        let forwarder = ToOwner {
            settings: Settings::new(
                SettingsFiles::of(&home, &version),
                Product::Collie,
                tasks.child(),
            ),
            home,
            version,
            clock: Clock::system(),
        };
        let answering = async {
            let (reader, mut writer) = listener.accept().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            lines.next_line().await.unwrap().unwrap();
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}\n")
                .await
                .unwrap();
            let line = lines.next_line().await.unwrap().unwrap();
            writer
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"read\":true}}\n")
                .await
                .unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        };
        let reading = forwarder
            .read(
                &owner,
                Product::Collie,
                "run.progress",
                json!({ "run_id": "elsewhere" }),
                &tasks,
            )
            .instrument(tracing::info_span!(SERVING));
        let reading = async {
            if raw {
                crate::trace::raw_call(reading).await
            } else {
                reading.await
            }
        };
        let (read, answer) = tokio::join!(answering, reading);
        assert_eq!(answer, Some(Ok(json!({ "read": true }))));
        read
    })
}

/// Covers: story:3-146
#[test]
fn a_read_forwarded_to_the_runs_owner_carries_its_client_spans_trace_context() {
    let env = TestEnv::new();

    let (read, spans) = exported(1, || forwarded_read(&env, false));

    let serving = the_span(&spans, &SpanKind::Internal, SERVING);
    let client = the_span(&spans, &SpanKind::Client, "run.progress");
    assert_eq!(
        read["params"]["_meta"],
        json!({
            "traceparent": format!(
                "00-{}-{}-01",
                client.span_context.trace_id(),
                client.span_context.span_id()
            ),
        })
    );
    assert_eq!(
        (client.parent_span_id, client.span_context.trace_id()),
        (
            serving.span_context.span_id(),
            serving.span_context.trace_id()
        )
    );
}

/// A raw call's read reaches the owner marked raw, so the owner's Run
/// trace records it as one (ADR 0099), even with no Trace context to send.
/// Covers: story:11-181
#[test]
fn a_raw_calls_read_forwarded_to_the_runs_owner_keeps_its_raw_mark() {
    let env = TestEnv::new();

    let read = forwarded_read(&env, true);

    assert_eq!(read["params"]["_meta"], json!({ "raw": true }));
}
