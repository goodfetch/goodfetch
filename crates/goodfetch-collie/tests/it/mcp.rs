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
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
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

/// Covers: story:11-234
#[test]
fn mcp_stdio_serves_embedded_docs_and_skills_matching_skill_install_digests() {
    use std::collections::BTreeMap;

    use goodfetch_mcp::resources::{LLMS_TXT_URI, SKILL_TREE_DIGEST_META_KEY};
    use rmcp::model::{
        GetSkillRequestParams, ReadResourceRequestParams, ResourceContents, SkillResources,
    };

    let env = TestEnv::new();
    let installed = goodfetch_test_support::cli(&env, COLLIE)
        .args(["skill", "install", "--json"])
        .json("SkillsInstalled")
        .unwrap();
    let installed_digests: BTreeMap<String, String> = installed.documents[0]["installed"]
        .as_array()
        .expect("installed array")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap().to_owned(),
                entry["digest"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_ne!(installed_digests.len(), 0);

    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let mut cmd = tokio::process::Command::new(COLLIE);
        cmd.env_clear()
            .envs(env.env_os())
            .args(["mcp", "--attached", "--profile", "code"]);
        let client =
            ().serve(TokioChildProcess::new(cmd).expect("spawn mcp --profile code"))
                .await
                .expect("rmcp initialized");

        assert_eq!(
            client.list_all_tools().await.expect("list_all_tools").len(),
            0
        );

        let resources = client.list_resources(None).await.expect("list_resources");
        assert!(resources.resources.iter().any(|r| r.uri == LLMS_TXT_URI));
        assert!(
            resources
                .resources
                .iter()
                .any(|r| r.uri == "goodfetch://docs/runs.md")
        );

        let llms = client
            .read_resource(ReadResourceRequestParams::new(LLMS_TXT_URI))
            .await
            .expect("read llms.txt");
        let ResourceContents::TextResourceContents { text, .. } = &llms.contents[0] else {
            panic!("expected text llms.txt");
        };
        assert!(text.contains("goodfetch://docs/runs.md"));

        let listed = client.list_skills(None).await.expect("list_skills");
        let mut served_digests = BTreeMap::new();
        for skill in &listed.skills {
            skill.validate().expect("SEP-2640 skill validation");
            let fetched = client
                .get_skill(GetSkillRequestParams::new(skill.uri.clone()))
                .await
                .expect("get_skill");
            assert_eq!(fetched.skill, *skill);

            let tree_digest = skill
                .meta
                .as_ref()
                .and_then(|m| m.get(SKILL_TREE_DIGEST_META_KEY))
                .and_then(Value::as_str)
                .expect("goodfetch_digest metadata")
                .to_owned();
            served_digests.insert(skill.name().to_owned(), tree_digest);

            let SkillResources::Files(ref files) = skill.resources else {
                panic!("expected static skill files with digests");
            };
            for file in files {
                let read = client
                    .read_resource(ReadResourceRequestParams::new(file.uri.clone()))
                    .await
                    .expect("read skill file");
                let ResourceContents::TextResourceContents { text, .. } = &read.contents[0] else {
                    panic!("expected text skill resource");
                };
                file.verify(text.as_bytes())
                    .expect("skill file digest verification");
            }
        }
        assert_eq!(served_digests, installed_digests);

        let _ = client.cancel().await;
    });
}

/// Covers: story:11-220, command:mcp
#[test]
fn mcp_http_startup_record_pure_stdout_and_endpoint_table() {
    use std::io::{BufRead as _, BufReader, Read as _};
    use std::net::SocketAddr;

    use goodfetch_test_support::{Endpoint, EndpointRunner, Listener, ListenerKind};
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let env = TestEnv::new();
    // Turn logging up to `trace` via user config so any stray log line on
    // stdout is caught (ADR 0070).
    let config_dir = env.home().join("0.0");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings.log]\nlevel = \"trace\"\n",
    )
    .unwrap();

    // Helper to launch `goodfetch-collie mcp --http --attached` and read its one stdout record,
    // draining stderr on a reader thread so trace logs never block the pipe.
    let spawn_http = |env: &TestEnv| {
        let mut command = std::process::Command::new(COLLIE);
        command
            .env_clear()
            .envs(env.env_os())
            .args(["mcp", "--http", "--attached"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let process = env.spawn(&mut command).unwrap();
        let stdout = process.child().stdout.take().unwrap();
        let stderr = process.child().stderr.take().unwrap();
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            let _ = reader.read_to_string(&mut buf);
            buf
        });
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap();
        assert!(read > 0, "mcp --http must print startup record on stdout");
        let record: Value = serde_json::from_str(line.trim()).expect("valid JSON startup record");
        let address = record["address"]
            .as_str()
            .expect("address field")
            .to_owned();
        let token = record["token"].as_str().expect("token field").to_owned();
        (process, reader, stderr_thread, address, token)
    };

    // Launch once to get an expired per-launch token from an earlier launch.
    let (earlier, _earlier_stdout, earlier_stderr, _earlier_addr, expired_token) = spawn_http(&env);
    let _ = earlier.child().kill();
    let _ = earlier.child().wait();
    let _ = earlier_stderr.join();

    // Launch the server under test.
    let (process, mut stdout_reader, stderr_thread, url, token) = spawn_http(&env);
    assert_ne!(
        token, expired_token,
        "each launch generates a fresh random token"
    );
    assert_eq!(token.len(), 64);
    assert!(
        url.starts_with("http://127.0.0.1:") && url.ends_with("/mcp"),
        "loopback default expected: {url}"
    );
    let parsed_url = url::Url::parse(&url).unwrap();
    let socket_addr: SocketAddr = format!(
        "{}:{}",
        parsed_url.host_str().unwrap(),
        parsed_url.port().unwrap()
    )
    .parse()
    .unwrap();

    // Run every applicable row of the endpoint table (`mcp_http`) and verify Audit log entries.
    let listener = Listener::new(
        ListenerKind::McpHttp,
        Endpoint::McpHttp {
            address: socket_addr,
        },
        token.clone(),
    )
    .with_other_listener_token("other-listener-token-00000000000000000000000000000000000000000000")
    .with_expired_token(expired_token);

    let ran = EndpointRunner::new(crate::endpoint::audit_verifier(&env, "mcp_http")).run(&listener);
    assert_eq!(ran, Ok(()), "EndpointRunner failed on mcp_http: {ran:?}");

    // Verify a real `rmcp` Streamable HTTP client connects and executes tools.
    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let http = reqwest_client(&[]);
        let transport = StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
        );
        let client = ().serve(transport).await.expect("rmcp http initialized");
        let tools = client.list_all_tools().await.expect("list_all_tools");
        assert!(tools.iter().any(|t| t.name == "run_start"));
        let started = client
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("run_start over HTTP");
        assert_ne!(started.is_error, Some(true));
        let run_id = started
            .structured_content
            .as_ref()
            .and_then(|v| v["run_id"].as_str())
            .expect("run_id")
            .to_owned();
        let mut finish_args = Map::new();
        finish_args.insert("run_id".to_owned(), Value::String(run_id));
        let finished = client
            .call_tool(CallToolRequestParams::new("run_finish").with_arguments(finish_args))
            .await
            .expect("run_finish over HTTP");
        assert_ne!(finished.is_error, Some(true));
        let _ = client.cancel().await;
    });

    // Stop server and assert stdout carried NOTHING beyond the single startup record,
    // while trace logs went to stderr.
    let _ = process.child().kill();
    let _ = process.child().wait();
    let mut trailing = String::new();
    stdout_reader.read_to_string(&mut trailing).unwrap();
    let stderr = stderr_thread.join().unwrap();
    assert!(
        trailing.is_empty(),
        "stdout must carry nothing after the startup record: {trailing:?}"
    );
    assert!(
        !stderr.is_empty(),
        "trace logs must go to stderr, never stdout"
    );
}

/// Covers: story:11-231
#[test]
fn mcp_http_two_concurrent_sessions_run_independently_on_binary() {
    use std::io::{BufRead as _, BufReader};

    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let env = TestEnv::new();
    let mut command = std::process::Command::new(COLLIE);
    command
        .env_clear()
        .envs(env.env_os())
        .args(["mcp", "--http", "--attached"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).unwrap();
    let stdout = process.child().stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let record: Value = serde_json::from_str(line.trim()).unwrap();
    let url = record["address"].as_str().unwrap().to_owned();
    let token = record["token"].as_str().unwrap().to_owned();

    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client_a = ()
            .serve(StreamableHttpClientTransport::with_client(
                reqwest_client(&[]),
                StreamableHttpClientTransportConfig::with_uri(url.clone())
                    .auth_header(token.clone()),
            ))
            .await
            .expect("client A initialized");

        let client_b = ()
            .serve(StreamableHttpClientTransport::with_client(
                reqwest_client(&[]),
                StreamableHttpClientTransportConfig::with_uri(url).auth_header(token),
            ))
            .await
            .expect("client B initialized");

        let (res_a, res_b) = tokio::join!(
            client_a.call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new())),
            client_b.call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new())),
        );
        let run_a = res_a
            .expect("session A run_start")
            .structured_content
            .unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let run_b = res_b
            .expect("session B run_start")
            .structured_content
            .unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(run_a, run_b);

        // Cancel session A while session B finishes its run unaffected.
        let _ = client_a.cancel().await;
        let mut finish_b = Map::new();
        finish_b.insert("run_id".to_owned(), Value::String(run_b));
        let finished_b = client_b
            .call_tool(CallToolRequestParams::new("run_finish").with_arguments(finish_b))
            .await
            .expect("session B run_finish succeeds after session A closes");
        assert_ne!(finished_b.is_error, Some(true));

        let _ = client_b.cancel().await;
    });

    let _ = process.child().kill();
    let _ = process.child().wait();
}

fn reqwest_client(extra_roots: &[rustls::pki_types::CertificateDer<'static>]) -> reqwest::Client {
    let mut roots = rustls::RootCertStore::empty();
    for cert in extra_roots {
        roots.add(cert.clone()).unwrap();
    }
    let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .no_proxy()
        .build()
        .unwrap()
}

/// Covers: story:11-221, setting:mcp.oauth.issuer, setting:mcp.oauth.audience, setting:daemon.tls.cert_file, setting:daemon.tls.key_file, error:setting_invalid
#[test]
fn mcp_http_non_loopback_refuses_without_tls_or_oauth_keys() {
    let run_remote = |config_toml: &str| {
        let env = TestEnv::new();
        if !config_toml.is_empty() {
            env.user_configuration(config_toml);
        }
        std::process::Command::new(COLLIE)
            .env_clear()
            .envs(env.env_os())
            .args(["mcp", "--http", "0.0.0.0:0", "--attached"])
            .output()
            .unwrap()
    };

    // 1. No TLS at all -> refused with `setting_invalid` naming `daemon.tls.*`.
    let no_tls = run_remote("");
    assert!(!no_tls.status.success());
    let stderr = String::from_utf8_lossy(&no_tls.stderr);
    assert!(stderr.contains("setting_invalid"), "stderr: {stderr}");
    assert!(stderr.contains("daemon.tls.cert_file"), "stderr: {stderr}");
    assert!(stderr.contains("daemon.tls.key_file"), "stderr: {stderr}");

    // 2. TLS files set, missing `mcp.oauth.issuer` -> refused naming `mcp.oauth.issuer`.
    let env_tls = TestEnv::new();
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .self_signed(&key_pair)
        .unwrap();
    let cert_path = env_tls.folder().join("mcp.crt");
    let key_path = env_tls.folder().join("mcp.key");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();

    let no_issuer = run_remote(&format!(
        "daemon.tls.cert_file = \"{}\"\ndaemon.tls.key_file = \"{}\"\n",
        cert_path.display(),
        key_path.display(),
    ));
    assert!(!no_issuer.status.success());
    let stderr = String::from_utf8_lossy(&no_issuer.stderr);
    assert!(stderr.contains("setting_invalid"), "stderr: {stderr}");
    assert!(stderr.contains("mcp.oauth.issuer"), "stderr: {stderr}");

    // 3. TLS + `mcp.oauth.issuer` set, missing `mcp.oauth.audience` -> refused naming `mcp.oauth.audience`.
    let no_audience = run_remote(&format!(
        "daemon.tls.cert_file = \"{}\"\n\
         daemon.tls.key_file = \"{}\"\n\
         mcp.oauth.issuer = \"https://auth.example.test\"\n",
        cert_path.display(),
        key_path.display(),
    ));
    assert!(!no_audience.status.success());
    let stderr = String::from_utf8_lossy(&no_audience.stderr);
    assert!(stderr.contains("setting_invalid"), "stderr: {stderr}");
    assert!(stderr.contains("mcp.oauth.audience"), "stderr: {stderr}");

    // 4. `daemon.tls.upstream = true` without OAuth keys -> refused naming `mcp.oauth.issuer`.
    let upstream_no_oauth = run_remote("daemon.tls.upstream = true\n");
    assert!(!upstream_no_oauth.status.success());
    let stderr = String::from_utf8_lossy(&upstream_no_oauth.stderr);
    assert!(stderr.contains("setting_invalid"), "stderr: {stderr}");
    assert!(stderr.contains("mcp.oauth.issuer"), "stderr: {stderr}");
}

/// Starts a minimal local OAuth 2.1 authorization server stub serving RFC 8414
/// metadata and an Ed25519 JWKS on loopback TCP, returning `(issuer, signing_key, kid, shutdown_tx)`.
#[expect(
    clippy::disallowed_types,
    reason = "integration test spawns a local loopback OAuth authorization server stub"
)]
fn start_local_oauth_stub(
    rt: &tokio::runtime::Runtime,
) -> (
    String,
    ed25519_dalek::SigningKey,
    String,
    tokio::sync::oneshot::Sender<()>,
) {
    use data_encoding::BASE64URL_NOPAD;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[0x5Au8; 32]);
    let x = BASE64URL_NOPAD.encode(signing_key.verifying_key().as_bytes());
    let kid = "collie-it-ed25519".to_owned();
    let (addr_tx, addr_rx) = std::sync::mpsc::channel();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let kid_clone = kid.clone();

    rt.spawn(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let issuer = format!("http://{addr}");
        let metadata = serde_json::to_string(&json!({
            "issuer": issuer,
            "jwks_uri": format!("{issuer}/jwks.json"),
        }))
        .unwrap();
        let jwks = serde_json::to_string(&json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "kid": kid_clone,
                "use": "sig",
                "alg": "EdDSA",
                "x": x,
            }]
        }))
        .unwrap();
        let _ = addr_tx.send(issuer);

        loop {
            let accepted = tokio::select! {
                _ = &mut stop_rx => break,
                res = listener.accept() => res,
            };
            let Ok((mut stream, _)) = accepted else {
                break;
            };
            let mut buf = [0u8; 4096];
            let Ok(n) = stream.read(&mut buf).await else {
                continue;
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            let (status, body) = match path {
                "/.well-known/oauth-authorization-server" => ("200 OK", metadata.as_str()),
                "/jwks.json" => ("200 OK", jwks.as_str()),
                _ => ("404 Not Found", "{}"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    let issuer = addr_rx.recv().expect("stub issuer address");
    (issuer, signing_key, kid, stop_tx)
}

fn sign_test_jwt(key: &ed25519_dalek::SigningKey, kid: &str, claims: &Value) -> String {
    use data_encoding::BASE64URL_NOPAD;
    use ed25519_dalek::Signer as _;
    let header = json!({ "alg": "EdDSA", "typ": "JWT", "kid": kid });
    let input = format!(
        "{}.{}",
        BASE64URL_NOPAD.encode(header.to_string().as_bytes()),
        BASE64URL_NOPAD.encode(claims.to_string().as_bytes()),
    );
    let sig = key.sign(input.as_bytes());
    format!("{input}.{}", BASE64URL_NOPAD.encode(&sig.to_bytes()))
}

/// Covers: story:11-221, setting:mcp.oauth.issuer, setting:mcp.oauth.audience, setting:daemon.tls.cert_file, setting:daemon.tls.key_file
#[test]
fn mcp_http_remote_tls_oauth_validates_tokens_and_never_passes_through_to_scripted_engine() {
    use std::io::{BufRead as _, BufReader};

    use goodfetch_test_support::{SCRIPTED_DAEMON_TOKEN, Tool};
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let env = TestEnv::new();
    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    // 1. Start the Scripted engine as the major's Daemon (`--mode socket`),
    // recording every `hello` and RPC call to `record.jsonl`.
    let script_path = env.folder().join("mcp-oauth-script.json");
    let record_path = env.folder().join("mcp-oauth-record.jsonl");
    let script = json!({
        "description": "Accepts MCP Daemon hello with the Daemon token and one run.start call",
        "steps": [
            {
                "expect": {
                    "method": "hello",
                    "params": {
                        "protocol_version": "0.1",
                        "surface": "mcp",
                        "form": "tool",
                        "needs": "collie",
                        "product_version": env!("CARGO_PKG_VERSION"),
                        "token": SCRIPTED_DAEMON_TOKEN,
                        "log_level": "warn"
                    }
                }
            },
            {
                "reply": {
                    "result": {
                        "protocol_version": "0.1",
                        "product": "collie",
                        "product_version": env!("CARGO_PKG_VERSION"),
                        "engine_instance": "0123456789abcdef",
                        "max_calls_in_flight": 16,
                        "timeouts": []
                    }
                }
            },
            {
                "expect": {
                    "method": "events.subscribe",
                    "params": {}
                }
            },
            {
                "expect": {
                    "method": "$/credit",
                    "params": {
                        "id": 2,
                        "credits": 16
                    }
                }
            },
            {
                "expect": {
                    "method": "run.start",
                    "params": {}
                }
            },
            {
                "reply": {
                    "result": {
                        "run_id": "run-from-scripted-engine"
                    }
                }
            }
        ]
    });
    std::fs::write(&script_path, serde_json::to_vec_pretty(&script).unwrap()).unwrap();

    let scripted_bin = goodfetch_test_support::tools()
        .unwrap()
        .path(Tool::ScriptedEngine)
        .unwrap()
        .to_path_buf();
    let mut scripted_cmd = std::process::Command::new(scripted_bin);
    scripted_cmd
        .env_clear()
        .envs(env.env_os())
        .args([
            "--script",
            script_path.to_str().unwrap(),
            "--mode",
            "socket",
            "--record",
            record_path.to_str().unwrap(),
            "--token",
            SCRIPTED_DAEMON_TOKEN,
            "--product-version",
            env!("CARGO_PKG_VERSION"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let scripted_proc = env.spawn(&mut scripted_cmd).unwrap();
    let mut scripted_stdout = BufReader::new(scripted_proc.child().stdout.take().unwrap());
    let mut ready_line = String::new();
    assert!(
        scripted_stdout.read_line(&mut ready_line).unwrap() > 0,
        "scripted engine must announce readiness"
    );

    // 2. Start local OAuth 2.1 authorization server stub + self-signed TLS cert.
    let (issuer, signing_key, kid, stop_stub) = start_local_oauth_stub(&rt);
    let audience = "https://mcp.remote.test/mcp";

    let key_pair = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
        .unwrap()
        .self_signed(&key_pair)
        .unwrap();
    let cert_path = env.folder().join("mcp-remote.crt");
    let key_path = env.folder().join("mcp-remote.key");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();

    env.user_configuration(&format!(
        "daemon.tls.cert_file = \"{}\"\n\
         daemon.tls.key_file = \"{}\"\n\
         mcp.oauth.issuer = \"{issuer}\"\n\
         mcp.oauth.audience = \"{audience}\"\n",
        cert_path.display(),
        key_path.display(),
    ));

    // 3. Start `goodfetch-collie mcp --http 0.0.0.0:0 --daemon` (non-loopback + TLS + OAuth 2.1).
    let mut mcp_cmd = std::process::Command::new(COLLIE);
    mcp_cmd
        .env_clear()
        .envs(env.env_os())
        .args(["mcp", "--http", "0.0.0.0:0", "--daemon"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mcp_proc = env.spawn(&mut mcp_cmd).unwrap();
    let mut mcp_stdout = BufReader::new(mcp_proc.child().stdout.take().unwrap());
    let mut startup_line = String::new();
    assert!(
        mcp_stdout.read_line(&mut startup_line).unwrap() > 0,
        "remote mcp --http must print startup record"
    );
    let startup: Value = serde_json::from_str(startup_line.trim()).unwrap();
    let bound_url = startup["address"].as_str().unwrap();
    assert!(
        bound_url.starts_with("https://") && bound_url.ends_with("/mcp"),
        "expected https startup address: {bound_url}"
    );
    assert!(
        startup.get("token").is_none(),
        "remote OAuth MCP server must not emit a shared-secret launch token: {startup}"
    );
    let port = url::Url::parse(bound_url).unwrap().port().unwrap();

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let valid_token = sign_test_jwt(
        &signing_key,
        &kid,
        &json!({
            "iss": issuer,
            "aud": audience,
            "exp": now_secs + 600,
            "sub": "remote-mcp-user"
        }),
    );

    // 4. Exercise RFC 9728 metadata, token rejections, and a real `rmcp` HTTPS call.
    rt.block_on(async {
        let https = reqwest_client(&[cert.der().clone()]);
        let base = format!("https://localhost:{port}");
        let mcp_url = format!("{base}/mcp");

        // RFC 9728 metadata names the configured issuer and audience.
        let meta_resp = https
            .get(format!("{base}/.well-known/oauth-protected-resource"))
            .send()
            .await
            .unwrap();
        assert_eq!(meta_resp.status(), 200);
        let meta: Value = meta_resp.json().await.unwrap();
        assert_eq!(meta["resource"], audience);
        assert_eq!(meta["authorization_servers"], json!([issuer]));
        assert_eq!(meta["bearer_methods_supported"], json!(["header"]));

        let init_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "remote-oauth-binary-test", "version": "1.0.0" }
            }
        });

        // Missing token -> 401 with resource_metadata.
        let missing = https
            .post(&mcp_url)
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body)
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 401);
        assert!(
            missing
                .headers()
                .get("www-authenticate")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("resource_metadata=")
        );

        // Wrong issuer, wrong audience, and expired tokens -> 401.
        for bad_claims in [
            json!({ "iss": "https://wrong-issuer.invalid", "aud": audience, "exp": now_secs + 600 }),
            json!({ "iss": issuer, "aud": "https://wrong-audience.invalid/mcp", "exp": now_secs + 600 }),
            json!({ "iss": issuer, "aud": audience, "exp": now_secs - 60 }),
        ] {
            let bad = sign_test_jwt(&signing_key, &kid, &bad_claims);
            let resp = https
                .post(&mcp_url)
                .header("Authorization", format!("Bearer {bad}"))
                .header("Accept", "application/json, text/event-stream")
                .json(&init_body)
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 401, "must refuse claims {bad_claims}");
        }

        // Valid OAuth token -> real `rmcp` Streamable HTTP client calls `run_start`.
        let client = ()
            .serve(StreamableHttpClientTransport::with_client(
                https,
                StreamableHttpClientTransportConfig::with_uri(mcp_url)
                    .auth_header(valid_token.clone()),
            ))
            .await
            .expect("rmcp HTTPS client initialized");
        let started = client
            .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
            .await
            .expect("run_start via remote OAuth MCP server");
        assert_ne!(started.is_error, Some(true));
        assert_eq!(
            started.structured_content.as_ref().unwrap()["run_id"],
            "run-from-scripted-engine"
        );
        let _ = client.cancel().await;
    });

    // 5. Stop MCP server and verify the Scripted engine exited cleanly and
    // NEVER received any part of the OAuth bearer token in `hello` or tool calls.
    let _ = mcp_proc.child().kill();
    let _ = mcp_proc.child().wait();
    let scripted_status = scripted_proc.child().wait().unwrap();
    assert!(
        scripted_status.success(),
        "scripted engine must play hello + run.start cleanly: {scripted_status}"
    );
    let _ = stop_stub.send(());

    let recorded = std::fs::read_to_string(&record_path).unwrap();
    assert!(
        recorded.contains("\"hello\""),
        "record must include hello: {recorded}"
    );
    assert!(
        recorded.contains("\"run.start\""),
        "record must include run.start: {recorded}"
    );
    assert!(
        recorded.contains(SCRIPTED_DAEMON_TOKEN),
        "hello must authenticate to Daemon with the Daemon's own token"
    );
    for segment in std::iter::once(valid_token.as_str()).chain(valid_token.split('.')) {
        assert!(
            !recorded.contains(segment),
            "OAuth bearer token segment leaked to Scripted engine: {recorded}"
        );
    }
}
