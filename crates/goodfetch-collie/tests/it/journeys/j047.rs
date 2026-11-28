//! J-047, "Connect an MCP host over Streamable HTTP using the startup record",
//! on the `mcp-http` surface (ADRs 0011, 0017, 0063, 0067, 0070; ticket #1365).
#![expect(
    clippy::disallowed_methods,
    reason = "Journey test starts goodfetch-collie mcp --http and drives an rmcp Streamable HTTP client"
)]

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Stdio;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, real_time_runtime};
use rmcp::ServiceExt as _;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Covers: story:11-220
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn mcp_http() {
    let mut run = JourneyRun::start(
        "J-047",
        "mcp-http",
        Path::new(COLLIE),
        ResultDocument::journey,
    )
    .expect("J-047's mcp-http variant starts");

    let mut process = None;
    let mut stdout_reader: Option<BufReader<std::process::ChildStdout>> = None;
    let mut startup: Option<Value> = None;
    let mut server_name = String::new();
    let mut tool_names: Vec<String> = Vec::new();
    let mut trailing_stdout = String::new();

    run.step(
        "Start `goodfetch-collie mcp --http --attached` and read the single `{ address, token }` JSON record from stdout",
        |run| {
            let mut command = run.env().process_command(run.engine());
            command
                .args(["mcp", "--http", "--attached"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let spawned = run.env().spawn(&mut command).unwrap();
            let stdout = spawned.child().stdout.take().unwrap();
            let mut reader = BufReader::new(stdout);
            let mut first_line = String::new();
            let read = reader.read_line(&mut first_line).unwrap();
            assert!(read > 0, "mcp --http must print startup record on stdout");
            let parsed: Value = serde_json::from_str(first_line.trim()).unwrap();
            startup = Some(parsed);
            stdout_reader = Some(reader);
            process = Some(spawned);
        },
    );

    run.step(
        "Build an MCP host configuration from `{ address, token }`, connect an `rmcp` Streamable HTTP client with it, and list tools",
        |_run| {
            let record = startup.as_ref().expect("startup record");
            // Standard MCP host configuration filled directly from `{ address, token }`.
            let host_config = json!({
                "mcpServers": {
                    "goodfetch": {
                        "type": "http",
                        "url": record["address"].as_str().unwrap(),
                        "headers": {
                            "Authorization": format!("Bearer {}", record["token"].as_str().unwrap())
                        }
                    }
                }
            });
            let url = host_config["mcpServers"]["goodfetch"]["url"]
                .as_str()
                .unwrap()
                .to_owned();
            let bearer = host_config["mcpServers"]["goodfetch"]["headers"]["Authorization"]
                .as_str()
                .unwrap()
                .strip_prefix("Bearer ")
                .unwrap()
                .to_owned();

            let (name, tools) = real_time_runtime().block_on(async {
                let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();
                let http = reqwest::Client::builder()
                    .tls_backend_preconfigured(tls)
                    .no_proxy()
                    .build()
                    .unwrap();
                let transport = StreamableHttpClientTransport::with_client(
                    http,
                    StreamableHttpClientTransportConfig::with_uri(url).auth_header(bearer),
                );
                let client = ().serve(transport).await.expect("rmcp http initialized");
                let name = client
                    .peer_info()
                    .and_then(|info| info.server_info.as_ref().map(|s| s.name.clone()))
                    .unwrap_or_default();
                let tools: Vec<String> = client
                    .list_all_tools()
                    .await
                    .expect("list_all_tools")
                    .into_iter()
                    .map(|tool| tool.name.into_owned())
                    .collect();
                let _ = client.cancel().await;
                (name, tools)
            });
            server_name = name;
            tool_names = tools;
        },
    );

    run.step(
        "Stop the MCP HTTP server cleanly and confirm stdout carried nothing beyond the startup record",
        |_run| {
            if let Some(spawned) = process.take() {
                let _ = spawned.child().kill();
                let _ = spawned.child().wait();
            }
            if let Some(mut reader) = stdout_reader.take() {
                use std::io::Read as _;
                let _ = reader.read_to_string(&mut trailing_stdout);
            }
        },
    );

    run.check(
        "The startup record names a loopback `http://127.0.0.1:<port>/mcp` address and a 64-character hex bearer token",
        |_| {
            let record = startup.as_ref().expect("startup record");
            let address = record["address"].as_str().unwrap();
            let token = record["token"].as_str().unwrap();
            assert!(
                address.starts_with("http://127.0.0.1:") && address.ends_with("/mcp"),
                "unexpected startup address: {address}"
            );
            assert_eq!(token.len(), 64);
            assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        },
    );

    run.check(
        "The MCP host connects with the configured bearer token, negotiates `goodfetch-collie`, and lists projected browser tools including `run_start`",
        |_| {
            assert_eq!(server_name, "goodfetch-collie");
            assert!(
                tool_names.iter().any(|name| name == "run_start"),
                "browser tools should include run_start: {tool_names:?}"
            );
        },
    );

    run.check(
        "No extra line or log byte appeared on stdout after the startup record",
        |_| {
            assert!(
                trailing_stdout.is_empty(),
                "unexpected trailing bytes on stdout: {trailing_stdout:?}"
            );
        },
    );

    run.finish()
        .expect("J-047's document is written")
        .assert_proved();
}
