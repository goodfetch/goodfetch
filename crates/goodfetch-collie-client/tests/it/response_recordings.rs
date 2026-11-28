//! Response recordings over a Daemon transport (ADRs 0105 and 0116; issue 1291):
//! the `response-recordings-daemon` client conformance case played by the
//! Scripted engine as the Daemon.
//! Proves:
//! - Replay mode reads the WARC file on the program's machine and sends its
//!   Base64 bytes in `recording_bytes` on `run.start` (so a Daemon works as a
//!   local engine does), and an unreadable file raises `recording_invalid`.
//! - `save_recording(path)` on `Run` and `RunView` streams `$/chunk` bytes
//!   under `$/credit` and writes the file atomically on the program's machine.

use std::path::Path;

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, ErrorCode};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::connection::HelloParams;
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::run::Recording;
use goodfetch_test_support::{
    RuntimeFile, TestEnv, Tool, case_logs, engine_script, real_time_runtime, reported_address,
    tools,
};
use serde_json::Value;

use crate::scripted_daemon::daemon_hello;

const SCRIPT: &str = "sdk-rust/run/response-recordings-daemon.json";

#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script and reads its runtime file before any async worker runs"
)]
fn scripted_daemon_with_paths(
    env: &TestEnv,
    script: &str,
    hello: &HelloParams,
    out_dir: &Path,
) -> (goodfetch_test_support::Process, DaemonEndpoint) {
    let input_path = out_dir.join("input.warc");
    let saved_path = out_dir.join("saved.warc");
    let view_path = out_dir.join("saved-view.warc");
    let text = std::fs::read_to_string(engine_script(script))
        .unwrap()
        .replace(
            &serde_json::to_string("/work/out/input.warc").unwrap(),
            &serde_json::to_string(&input_path.to_string_lossy()).unwrap(),
        )
        .replace(
            &serde_json::to_string("/work/out/saved.warc").unwrap(),
            &serde_json::to_string(&saved_path.to_string_lossy()).unwrap(),
        )
        .replace(
            &serde_json::to_string("/work/out/saved-view.warc").unwrap(),
            &serde_json::to_string(&view_path.to_string_lossy()).unwrap(),
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

/// The `response-recordings-daemon` client conformance case, played by the
/// Scripted engine as the Daemon.
/// Covers: story:11-161, story:11-162, operation:recording.save_as
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "test fixture setup and file assertions in an integration test"
)]
#[test]
fn response_recordings_daemon_uploads_replay_bytes_and_streams_save_recording() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "response-recordings-daemon",
        RustMode::Daemon,
    )
    .unwrap();
    let runtime = real_time_runtime();
    let env = TestEnv::new();
    let work = tempfile::tempdir().unwrap();
    let replay_file = work.path().join("input.warc");
    std::fs::write(&replay_file, b"WARC/1.1\r\n").unwrap();
    let hello = daemon_hello();
    let (process, endpoint) = scripted_daemon_with_paths(&env, SCRIPT, &hello, work.path());

    runtime.block_on(async {
        let (connection, _) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &Endpoint::Daemon(endpoint),
            hello,
        )
        .await
        .unwrap();

        // Unreadable replay file raises recording_invalid before sending anything.
        let missing = work.path().join("missing.warc");
        let missing_err = connection
            .start()
            .recording(Recording::replay(missing.to_string_lossy()))
            .await
            .unwrap_err();
        assert_eq!(
            missing_err.code(),
            ErrorCode::new("recording_invalid").unwrap()
        );

        // Replay start sends Base64 recording_bytes over the Daemon connection.
        let run = connection
            .start()
            .recording(Recording::replay(replay_file.to_string_lossy()))
            .await
            .unwrap();
        let out_path = work.path().join("saved.warc");
        let saved = run.save_recording(&out_path).await.unwrap();
        assert!(!saved.written);
        assert_eq!(std::fs::read(&out_path).unwrap(), b"WARC/1.1\r\n");

        let view = connection.open_run(run.id()).unwrap();
        let out_path2 = work.path().join("saved-view.warc");
        let saved2 = view.save_recording(&out_path2).await.unwrap();
        assert!(!saved2.written);
        assert_eq!(std::fs::read(&out_path2).unwrap(), b"WARC/1.1\r\n");
        view.close();

        run.finish().await.unwrap();
        connection.close().await.unwrap();
    });
    let status = process.child().wait().unwrap();
    case.check("the Daemon played its script through", || {
        assert!(status.success(), "scripted daemon exited with {status}");
    });
}
