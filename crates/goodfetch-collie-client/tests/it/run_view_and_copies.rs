//! `open_run(run_id)`, `RunView`, Key-value store `keys()` & schema-typed
//! values (`schemars` + hand-written JSON Schema), and `Checkpoint` /
//! `RunTrace` `save_as(path)` + `stream()` (ADRs 0082, 0102, 0103 and 0116;
//! issue 1284): the `run-view-and-copies` client conformance case played
//! against the Scripted engine as a Daemon.

use std::path::Path;

use goodfetch_collie_client::schemars::{self, JsonSchema};
use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{
    ClientConnection, Connection, ConnectionRuntime, Error, ErrorCode, RunView, Schema, StoredBytes,
};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::run::{Lifecycle, RunId};
use goodfetch_test_support::{
    RuntimeFile, TestEnv, Tool, case_logs, engine_script, real_time_runtime, reported_address,
    tools,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::scripted_daemon::daemon_hello;
use crate::start::{code_group_retryable, failure};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
struct Book {
    title: String,
    pages: u32,
}

/// Compile-time proof: a generic function typed with `&impl Connection` calls
/// `open_run` on either Product.
#[expect(
    dead_code,
    reason = "compile-time proof of Connection::open_run trait member"
)]
fn open_from_connection(connection: &impl Connection, run_id: &RunId) -> Result<RunView, Error> {
    connection.open_run(run_id)
}

async fn collect_chunks(mut stream: goodfetch_collie_client::EventStream<Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    out
}

async fn collect_strings(mut stream: goodfetch_collie_client::EventStream<String>) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        out.push(item.unwrap());
    }
    out
}

struct Played {
    schema_refused: Error,
    live_keys: Vec<String>,
    cp_saved: Vec<u8>,
    cp_streamed: Vec<u8>,
    trace_saved: Vec<u8>,
    trace_streamed: Vec<u8>,
    finished_progress_last: bool,
    finished_report_id: String,
    finished_trace_location: Option<String>,
    finished_book: Option<Book>,
    finished_manual: Option<Value>,
    finished_bytes: Option<StoredBytes>,
    finished_keys: Vec<String>,
    interrupted_lifecycle: Lifecycle,
    interrupted_error: Error,
    missing_error: Error,
}

async fn play_views(
    connection: &ClientConnection,
    book_schema: &Schema<Book>,
    manual_schema: &Schema<Value>,
) -> (
    bool,
    String,
    Option<String>,
    Option<Book>,
    Option<Value>,
    Option<StoredBytes>,
    Vec<String>,
    Lifecycle,
    Error,
    Error,
) {
    let finished_id = RunId::parse("finished-run").unwrap();
    let finished_view = connection.open_run(&finished_id).unwrap();
    let mut finished_progress = finished_view.progress();
    let mut finished_progress_last = false;
    while let Some(item) = finished_progress.next().await {
        finished_progress_last = item.unwrap().last;
    }
    let finished_report = finished_view.report().await.unwrap();
    let finished_trace = finished_view.trace().await.unwrap();
    let finished_kv = finished_view.key_value_store();
    let finished_book = finished_kv.get_with("item-a", book_schema).await.unwrap();
    let finished_manual = finished_kv
        .get_with("item-json", manual_schema)
        .await
        .unwrap();
    let finished_bytes = finished_kv.get_bytes("item-bin").await.unwrap();
    let finished_keys = collect_strings(finished_kv.keys(Some("item-"))).await;
    finished_view.close();

    let interrupted_id = RunId::parse("interrupted-run").unwrap();
    let interrupted_view = connection.open_run(&interrupted_id).unwrap();
    let mut interrupted_progress = interrupted_view.progress();
    let mut interrupted_lifecycle = Lifecycle::Running;
    while let Some(item) = interrupted_progress.next().await {
        interrupted_lifecycle = item.unwrap().lifecycle;
    }
    let interrupted_error = failure(interrupted_view.report().await);
    interrupted_view.close();

    let missing_id = RunId::parse("missing-run").unwrap();
    let missing_view = connection.open_run(&missing_id).unwrap();
    let missing_error = failure(missing_view.report().await);
    missing_view.close();

    (
        finished_progress_last,
        finished_report.run_id.as_str().to_owned(),
        finished_trace.map(|t| t.location().to_owned()),
        finished_book,
        finished_manual,
        finished_bytes,
        finished_keys,
        interrupted_lifecycle,
        interrupted_error,
        missing_error,
    )
}

async fn played(endpoint: DaemonEndpoint, hello: HelloParams, dir: &Path) -> Played {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        hello,
    )
    .await
    .unwrap();

    let run = connection.start().await.unwrap();
    let kv = run.key_value_store();

    let book_schema = Schema::<Book>::derived();
    let manual_schema = Schema::json(json!({
        "type": "object",
        "required": ["kind", "count"],
        "properties": {
            "kind": { "type": "string" },
            "count": { "type": "integer", "minimum": 1 }
        }
    }))
    .unwrap();

    // Schema refusal happens locally before sending anything on the wire.
    let schema_refused = failure(
        kv.set_with(
            "bad-manual",
            &json!({ "kind": "manual", "count": 0 }),
            &manual_schema,
        )
        .await,
    );

    kv.set_with(
        "item-a",
        &Book {
            title: "Dune".to_owned(),
            pages: 412,
        },
        &book_schema,
    )
    .await
    .unwrap();
    kv.set_with(
        "item-json",
        &json!({ "kind": "manual", "count": 2 }),
        &manual_schema,
    )
    .await
    .unwrap();
    kv.set_bytes("item-bin", b"hello".to_vec(), "application/octet-stream")
        .await
        .unwrap();
    let live_keys = collect_strings(kv.keys(Some("item-"))).await;

    let checkpoint = run.checkpoint().await.unwrap();
    let cp_path = dir.join("nested").join("cp.sqlite");
    checkpoint.save_as(&cp_path).await.unwrap();
    let cp_streamed = collect_chunks(checkpoint.stream()).await;

    let trace = run.trace().await.unwrap().expect("trace present");
    let trace_path = dir.join("nested").join("trace.zip");
    trace.save_as(&trace_path).await.unwrap();
    let trace_streamed = collect_chunks(trace.stream()).await;

    run.finish().await.unwrap();

    let (
        finished_progress_last,
        finished_report_id,
        finished_trace_location,
        finished_book,
        finished_manual,
        finished_bytes,
        finished_keys,
        interrupted_lifecycle,
        interrupted_error,
        missing_error,
    ) = play_views(&connection, &book_schema, &manual_schema).await;

    connection.close().await.unwrap();

    Played {
        schema_refused,
        live_keys,
        cp_saved: read_saved(&cp_path),
        cp_streamed,
        trace_saved: read_saved(&trace_path),
        trace_streamed,
        finished_progress_last,
        finished_report_id,
        finished_trace_location,
        finished_book,
        finished_manual,
        finished_bytes,
        finished_keys,
        interrupted_lifecycle,
        interrupted_error,
        missing_error,
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the tiny saved files after the Run finishes"
)]
fn read_saved(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script and reads its runtime file before any async worker runs"
)]
fn scripted_daemon_with_save_paths(
    env: &TestEnv,
    script: &str,
    hello: &HelloParams,
    out_dir: &Path,
) -> (goodfetch_test_support::Process, DaemonEndpoint) {
    let cp_path = out_dir.join("nested").join("cp.sqlite");
    let trace_path = out_dir.join("nested").join("trace.zip");
    let text = std::fs::read_to_string(engine_script(script))
        .unwrap()
        .replace(
            &serde_json::to_string("/work/out/cp.sqlite").unwrap(),
            &serde_json::to_string(&cp_path.to_string_lossy()).unwrap(),
        )
        .replace(
            &serde_json::to_string("/work/out/trace.zip").unwrap(),
            &serde_json::to_string(&trace_path.to_string_lossy()).unwrap(),
        );
    let mut played: Value = serde_json::from_str(&text).unwrap();
    played["steps"][0]["expect"]["params"] = serde_json::to_value(hello).unwrap();
    let path = env.folder().join("script.json");
    std::fs::write(&path, serde_json::to_vec(&played).unwrap()).unwrap();
    let scripted = tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_owned();
    let mut command = env.process_command(scripted);
    command
        .arg("--script")
        .arg(&path)
        .args(["--mode", "socket"])
        .stderr(std::process::Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let reported = reported_address(&mut process.child()).unwrap();
    let runtime_file = reported.line["runtime_file"].as_str().unwrap();
    let found = RuntimeFile::read(runtime_file.as_ref()).unwrap();
    let endpoint = serde_json::from_value(serde_json::to_value(found.endpoint).unwrap()).unwrap();
    (process, endpoint)
}

// Covers: story:11-130, operation:run.trace, operation:checkpoint.save_as, operation:checkpoint.stream, operation:trace.save_as, operation:trace.stream, operation:key_value_store.keys
#[test]
fn run_view_and_copies_on_a_daemon() {
    let env = TestEnv::new();
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "run-view-and-copies",
        RustMode::Daemon,
    )
    .unwrap();
    let hello = daemon_hello();
    let (daemon, endpoint) = scripted_daemon_with_save_paths(
        &env,
        "sdk-rust/run/run-view-and-copies.json",
        &hello,
        env.home(),
    );
    let result = real_time_runtime().block_on(played(endpoint, hello, env.home()));
    let status = daemon.child().wait().unwrap();
    assert!(status.success(), "scripted daemon exited with {status}");

    case.check(
        "hand-written JSON Schema rejects invalid value locally with value_invalid",
        || {
            assert_eq!(
                code_group_retryable(&result.schema_refused),
                (ErrorCode::VALUE_INVALID, ErrorGroup::Program, false)
            );
        },
    );
    case.check(
        "KV keys() yields prefixed keys in order on live Run and RunView",
        || {
            assert_eq!(result.live_keys, vec!["item-a", "item-bin", "item-json"]);
            assert_eq!(
                result.finished_keys,
                vec!["item-a", "item-bin", "item-json"]
            );
        },
    );
    case.check(
        "Checkpoint.save_as and RunTrace.save_as write whole and stream() delivers the same bytes",
        || {
            assert_eq!(result.cp_saved, b"cp-bytes!!");
            assert_eq!(result.cp_streamed, b"cp-bytes!!");
            assert_eq!(result.trace_saved, b"trace-bytes!");
            assert_eq!(result.trace_streamed, b"trace-bytes!");
        },
    );
    case.check(
        "RunView on finished Run reads progress, report, trace, schemars & JSON Schema KV values, and bytes",
        || {
            assert!(result.finished_progress_last);
            assert_eq!(result.finished_report_id, "finished-run");
            assert_eq!(
                result.finished_trace_location.as_deref(),
                Some("/engine/finished-trace.zip")
            );
            assert_eq!(
                result.finished_book,
                Some(Book {
                    title: "Dune".to_owned(),
                    pages: 412,
                })
            );
            assert_eq!(
                result.finished_manual,
                Some(json!({ "kind": "manual", "count": 2 }))
            );
            assert_eq!(
                result.finished_bytes.as_ref().map(|b| (b.content.as_slice(), b.content_type.as_str())),
                Some((b"hello".as_slice(), "application/octet-stream"))
            );
        },
    );
    case.check(
        "RunView on interrupted Run raises run_interrupted and unknown ID raises run_not_found",
        || {
            assert_eq!(result.interrupted_lifecycle, Lifecycle::Stopped);
            assert_eq!(
                code_group_retryable(&result.interrupted_error),
                (ErrorCode::RUN_INTERRUPTED, ErrorGroup::Unavailable, false)
            );
            assert_eq!(
                code_group_retryable(&result.missing_error),
                (ErrorCode::RUN_NOT_FOUND, ErrorGroup::Program, false)
            );
        },
    );
}
