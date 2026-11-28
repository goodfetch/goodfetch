//! Integration tests for `goodfetch-collie mcp` over stdio (ADR 0070, ticket
//! #1359): verifies `initialize` server identity and instructions, strict
//! JSON-RPC stdout framing under `trace` logging (`StrictStdout`), real tool
//! calls returning `structuredContent` + JSON text, and typed Error codes
//! returned with `isError: true`, CLI's `CommandError` and `fix` first.
#![expect(
    clippy::disallowed_methods,
    reason = "integration test spawns the Collie binary and writes its temp config file"
)]
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end stdio JSON-RPC session exercises initialize, list, success call, error call, and clean EOF"
)]

use std::io::Write as _;
use std::process::Stdio;
use std::time::Duration;

use goodfetch_test_support::{StrictStdout, TestEnv};
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use serde_json::{Map, Value, json};

use crate::COLLIE;

/// Covers: story:11-222, story:11-229, command:mcp
#[test]
fn mcp_stdio_strict_stdout_serves_tools_and_error_result_under_trace_logging() {
    let env = TestEnv::new();
    // Turn logging up to `trace` via user config so any stray log line on
    // stdout fails `StrictStdout` (ADR 0070).
    let config_dir = env.home().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.toml"), "[log]\nlevel = \"trace\"\n").unwrap();

    let mut command = std::process::Command::new(COLLIE);
    command
        .env_clear()
        .envs(env.env_os())
        .args(["mcp", "--attached"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let process = env.spawn(&mut command).unwrap();
    let mut stdout = StrictStdout::of(&process)
        .unwrap()
        .within(Duration::from_secs(15));
    let mut stdin = process.child().stdin.take().unwrap();

    let send = |stdin: &mut std::process::ChildStdin, value: &Value| {
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        stdin.write_all(&bytes).unwrap();
        stdin.flush().unwrap();
    };

    // 1. initialize
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "strict-stdout-test", "version": "1.0.0" }
            }
        }),
    );
    let init = stdout
        .try_next()
        .expect("strict JSON-RPC line on stdout")
        .expect("initialize reply");
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "goodfetch-collie");
    let instructions = init["result"]["instructions"]
        .as_str()
        .expect("instructions present");
    assert!(instructions.contains("browser"));

    // 2. notifications/initialized
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );

    // 3. tools/list
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    );
    let listed = stdout
        .try_next()
        .expect("strict JSON-RPC line on stdout")
        .expect("tools/list reply");
    assert_eq!(listed["id"], 2);
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(
        tools.iter().any(|tool| tool["name"] == "run_start"),
        "browser profile includes run_start"
    );

    // 4. Call run_start -> succeeds with structuredContent + JSON text
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "run_start",
                "arguments": {}
            }
        }),
    );
    let started = stdout
        .try_next()
        .expect("strict JSON-RPC line on stdout")
        .expect("run_start reply");
    assert_eq!(started["id"], 3);
    assert_ne!(started["result"]["isError"], Value::Bool(true));
    let run_id = started["result"]["structuredContent"]["run_id"]
        .as_str()
        .expect("structuredContent carries run_id");
    assert_ne!(run_id, "");
    let text_body = started["result"]["content"][0]["text"]
        .as_str()
        .expect("JSON text block");
    let parsed_body: Value = serde_json::from_str(text_body).unwrap();
    assert_eq!(parsed_body["run_id"], run_id);

    // 5. Call run_finish with an unknown run_id -> isError: true + CommandError + fix first
    send(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "run_finish",
                "arguments": { "run_id": "nonexistent-run-id" }
            }
        }),
    );
    let failed = stdout
        .try_next()
        .expect("strict JSON-RPC line on stdout")
        .expect("run_finish error reply");
    assert_eq!(failed["id"], 4);
    assert_eq!(failed["result"]["isError"], Value::Bool(true));
    let err_obj = &failed["result"]["structuredContent"];
    assert_eq!(err_obj["code"], "run_not_found");
    assert_eq!(err_obj["group"], "program");
    assert_eq!(err_obj["retryable"], Value::Bool(false));
    assert_eq!(err_obj["operation"], "run.finish");
    if let Some(fix) = err_obj["fix"].as_str() {
        let text = failed["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with(fix), "fix must come first: {text}");
    }

    // Close stdin cleanly and confirm no stray bytes follow on stdout
    drop(stdin);
    assert_eq!(
        stdout.try_next().expect("clean EOF on stdout"),
        None,
        "no trailing non-JSON-RPC output on stdout"
    );
    let stderr = stdout.stderr_to_end();
    assert!(
        !stderr.is_empty(),
        "trace logs went to stderr, never stdout"
    );
    let status = process.child().wait().unwrap();
    assert!(status.success(), "mcp server exited cleanly: {status}");
}

/// Covers: story:11-222, story:11-229, command:mcp
#[test]
fn mcp_stdio_rmcp_client_connects_and_calls_tools_on_binary() {
    let env = TestEnv::new();
    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let mut command = tokio::process::Command::new(COLLIE);
        command
            .env_clear()
            .envs(env.env_os())
            .args(["mcp", "--attached"]);

        let transport = TokioChildProcess::new(command).expect("spawn goodfetch-collie mcp");
        let client = ().serve(transport).await.expect("rmcp initialized");

        let info = client.peer_info().expect("server info");
        assert_eq!(
            info.server_info.as_ref().map(|s| s.name.as_str()),
            Some("goodfetch-collie")
        );
        assert!(info.instructions.is_some());

        let tools = client.list_all_tools().await.expect("list_all_tools");
        assert!(tools.iter().any(|t| t.name == "run_start"));

        let started = client
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("run_start succeeded");
        assert_ne!(started.is_error, Some(true));
        let run_id = started
            .structured_content
            .as_ref()
            .and_then(|v| v["run_id"].as_str())
            .expect("run_id returned")
            .to_owned();

        let mut finish_args = Map::new();
        finish_args.insert("run_id".to_owned(), Value::String(run_id));
        let finished = client
            .call_tool(CallToolRequestParams::new("run_finish").with_arguments(finish_args))
            .await
            .expect("run_finish succeeded");
        assert_ne!(finished.is_error, Some(true));

        let _ = client.cancel().await;
    });
}

/// Covers: story:11-223, story:11-224, setting:mcp.profile
#[test]
fn mcp_stdio_profile_flag_and_setting_select_profile() {
    let env = TestEnv::new();
    let config_dir = env.home().join("0.0");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.mcp]\nprofile = \"run\"\n",
    )
    .unwrap();

    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        // 1. Without --profile, `mcp.profile = "run"` from user configuration applies.
        let mut cmd_setting = tokio::process::Command::new(COLLIE);
        cmd_setting
            .env_clear()
            .envs(env.env_os())
            .args(["mcp", "--attached"]);
        let client_run =
            ().serve(TokioChildProcess::new(cmd_setting).expect("spawn mcp with setting"))
                .await
                .expect("rmcp initialized");
        let run_info = client_run.peer_info().expect("server info");
        assert!(
            run_info
                .instructions
                .as_deref()
                .is_some_and(|s| s.contains("`run`")),
            "instructions should name `run` profile: {:?}",
            run_info.instructions
        );
        let run_tools = client_run.list_all_tools().await.expect("list_all_tools");
        assert!(run_tools.iter().any(|t| t.name == "run_start"));
        assert!(
            !run_tools.iter().any(|t| t.name == "page_goto"),
            "run profile must not expose page tools"
        );
        let _ = client_run.cancel().await;

        // 2. `--profile code` overrides `mcp.profile` and offers no per-operation tools.
        let mut cmd_code = tokio::process::Command::new(COLLIE);
        cmd_code
            .env_clear()
            .envs(env.env_os())
            .args(["mcp", "--attached", "--profile", "code"]);
        let client_code =
            ().serve(TokioChildProcess::new(cmd_code).expect("spawn mcp --profile code"))
                .await
                .expect("rmcp initialized");
        let code_info = client_code.peer_info().expect("server info");
        assert!(
            code_info
                .instructions
                .as_deref()
                .is_some_and(|s| s.contains("`code`")),
            "instructions should name `code` profile: {:?}",
            code_info.instructions
        );
        let code_tools = client_code.list_all_tools().await.expect("list_all_tools");
        assert!(
            code_tools.is_empty(),
            "code profile must offer no per-operation tools"
        );
        let _ = client_code.cancel().await;
    });
}
