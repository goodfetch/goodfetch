//! J-048, "Expose a remote MCP server over TLS with OAuth 2.1 audience
//! enforcement", on the `mcp-http` surface (ADRs 0017, 0049, 0063, 0067, 0070;
//! ticket #1367).
#![expect(
    clippy::disallowed_methods,
    reason = "Journey test starts goodfetch-collie mcp --http and drives an rmcp Streamable HTTP HTTPS client"
)]
#![expect(
    clippy::disallowed_types,
    reason = "Journey test runs a local loopback OAuth 2.1 authorization server stub"
)]

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signer as _, SigningKey};
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::JourneyRun;
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Map, Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

fn sign_jwt(key: &SigningKey, kid: &str, claims: &Value) -> String {
    let header = json!({ "alg": "EdDSA", "typ": "JWT", "kid": kid });
    let input = format!(
        "{}.{}",
        BASE64URL_NOPAD.encode(header.to_string().as_bytes()),
        BASE64URL_NOPAD.encode(claims.to_string().as_bytes()),
    );
    let sig = key.sign(input.as_bytes());
    format!("{input}.{}", BASE64URL_NOPAD.encode(&sig.to_bytes()))
}

/// Covers: story:11-221
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn mcp_http() {
    let mut run = JourneyRun::start(
        "J-048",
        "mcp-http",
        Path::new(COLLIE),
        ResultDocument::journey,
    )
    .expect("J-048's mcp-http variant starts");

    let mut refused_exit = false;
    let mut refused_stderr = String::new();
    let mut startup: Option<Value> = None;
    let mut metadata: Option<Value> = None;
    let mut wrong_aud_status = 0u16;
    let mut expired_status = 0u16;
    let mut run_started = false;
    let mut issuer = String::new();
    let audience = "https://mcp.journey.test/mcp".to_owned();

    let mut process = None;
    let mut cert_der: Option<rustls::pki_types::CertificateDer<'static>> = None;
    let signing_key = SigningKey::from_bytes(&[0x6Cu8; 32]);
    let kid = "j048-ed25519".to_owned();
    let (stop_stub_tx, mut stop_stub_rx) = tokio::sync::oneshot::channel::<()>();
    let rt = goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    run.step(
        "Try starting `goodfetch-collie mcp --http 0.0.0.0:0 --attached` without TLS or OAuth settings and verify refusal with `setting_invalid`",
        |run| {
            let output = run
                .env()
                .process_command(run.engine())
                .args(["mcp", "--http", "0.0.0.0:0", "--attached"])
                .output()
                .unwrap();
            refused_exit = !output.status.success();
            refused_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        },
    );

    run.step(
        "Configure `daemon.tls.cert_file`, `daemon.tls.key_file`, `mcp.oauth.issuer`, and `mcp.oauth.audience` pointing at a local authorization server stub and start `goodfetch-collie mcp --http 0.0.0.0:0 --attached`",
        |run| {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

            let x = BASE64URL_NOPAD.encode(signing_key.verifying_key().as_bytes());
            let kid_clone = kid.clone();
            let (addr_tx, addr_rx) = std::sync::mpsc::channel();
            rt.spawn(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = listener.local_addr().unwrap();
                let iss = format!("http://{addr}");
                let meta_body = serde_json::to_string(&json!({
                    "issuer": iss,
                    "jwks_uri": format!("{iss}/jwks.json"),
                }))
                .unwrap();
                let jwks_body = serde_json::to_string(&json!({
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
                let _ = addr_tx.send(iss);
                loop {
                    let accepted = tokio::select! {
                        _ = &mut stop_stub_rx => break,
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
                        "/.well-known/oauth-authorization-server" => ("200 OK", meta_body.as_str()),
                        "/jwks.json" => ("200 OK", jwks_body.as_str()),
                        _ => ("404 Not Found", "{}"),
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                }
            });
            issuer = addr_rx.recv().unwrap();

            let key_pair = rcgen::KeyPair::generate().unwrap();
            let cert = rcgen::CertificateParams::new(vec![
                "localhost".to_owned(),
                "127.0.0.1".to_owned(),
            ])
            .unwrap()
            .self_signed(&key_pair)
            .unwrap();
            let cert_path = run.env().folder().join("j048.crt");
            let key_path = run.env().folder().join("j048.key");
            std::fs::write(&cert_path, cert.pem()).unwrap();
            std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();
            cert_der = Some(cert.der().clone());

            run.env().user_configuration(&format!(
                "daemon.tls.cert_file = \"{}\"\n\
                 daemon.tls.key_file = \"{}\"\n\
                 mcp.oauth.issuer = \"{issuer}\"\n\
                 mcp.oauth.audience = \"{audience}\"\n",
                cert_path.display(),
                key_path.display(),
            ));

            let mut command = run.env().process_command(run.engine());
            command
                .args(["mcp", "--http", "0.0.0.0:0", "--attached"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let spawned = run.env().spawn(&mut command).unwrap();
            let stdout = spawned.child().stdout.take().unwrap();
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            startup = Some(serde_json::from_str(line.trim()).unwrap());
            process = Some(spawned);
        },
    );

    run.step(
        "Fetch RFC 9728 protected-resource metadata over HTTPS, verify wrong-audience and expired tokens are refused with 401, and connect an `rmcp` Streamable HTTP client with a valid audience-bound OAuth 2.1 token to call `run_start`",
        |_run| {
            let record = startup.as_ref().expect("startup record");
            let port = url::Url::parse(record["address"].as_str().unwrap())
                .unwrap()
                .port()
                .unwrap();
            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der.clone().unwrap()).unwrap();
            let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
            let https = reqwest::Client::builder()
                .tls_backend_preconfigured(tls)
                .no_proxy()
                .build()
                .unwrap();

            let base = format!("https://localhost:{port}");
            let mcp_url = format!("{base}/mcp");
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let wrong_aud = sign_jwt(
                &signing_key,
                &kid,
                &json!({ "iss": issuer, "aud": "https://other.invalid/mcp", "exp": now_secs + 600 }),
            );
            let expired = sign_jwt(
                &signing_key,
                &kid,
                &json!({ "iss": issuer, "aud": audience, "exp": now_secs - 60 }),
            );
            let valid = sign_jwt(
                &signing_key,
                &kid,
                &json!({ "iss": issuer, "aud": audience, "exp": now_secs + 600, "sub": "j048" }),
            );

            rt.block_on(async {
                let meta_resp = https
                    .get(format!("{base}/.well-known/oauth-protected-resource"))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(meta_resp.status(), 200);
                metadata = Some(meta_resp.json().await.unwrap());

                let init_body = json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": {},
                        "clientInfo": { "name": "j048", "version": "1.0.0" }
                    }
                });
                wrong_aud_status = https
                    .post(&mcp_url)
                    .header("Authorization", format!("Bearer {wrong_aud}"))
                    .header("Accept", "application/json, text/event-stream")
                    .json(&init_body)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16();
                expired_status = https
                    .post(&mcp_url)
                    .header("Authorization", format!("Bearer {expired}"))
                    .header("Accept", "application/json, text/event-stream")
                    .json(&init_body)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16();

                let client = ()
                    .serve(StreamableHttpClientTransport::with_client(
                        https,
                        StreamableHttpClientTransportConfig::with_uri(mcp_url).auth_header(valid),
                    ))
                    .await
                    .expect("rmcp HTTPS client initialized");
                let started = client
                    .call_tool(CallToolRequestParams::new("run_start").with_arguments(Map::new()))
                    .await
                    .expect("run_start over HTTPS");
                run_started = started.is_error != Some(true)
                    && started
                        .structured_content
                        .as_ref()
                        .and_then(|v| v["run_id"].as_str())
                        .is_some_and(|id| !id.is_empty());
                let _ = client.cancel().await;
            });

            if let Some(spawned) = process.take() {
                let _ = spawned.child().kill();
                let _ = spawned.child().wait();
            }
            let _ = stop_stub_tx.send(());
        },
    );

    run.check(
        "Starting on a non-loopback address without TLS or OAuth settings exits non-zero with `setting_invalid` naming the missing key",
        |_| {
            assert!(refused_exit, "non-loopback without TLS must fail");
            assert!(
                refused_stderr.contains("setting_invalid")
                    && refused_stderr.contains("daemon.tls.cert_file"),
                "unexpected stderr: {refused_stderr}"
            );
        },
    );

    run.check(
        "The remote startup record announces an `https://` URL and omits any shared-secret `token` field",
        |_| {
            let record = startup.as_ref().expect("startup record");
            let address = record["address"].as_str().unwrap();
            assert!(
                address.starts_with("https://") && address.ends_with("/mcp"),
                "unexpected startup address: {address}"
            );
            assert!(
                record.get("token").is_none(),
                "remote OAuth server must not print a shared secret token: {record}"
            );
        },
    );

    run.check(
        "Protected-resource metadata names `mcp.oauth.issuer` and `mcp.oauth.audience`, wrong-audience and expired tokens receive HTTP 401, and the valid OAuth 2.1 token succeeds via `rmcp`",
        |_| {
            let meta = metadata.as_ref().expect("RFC 9728 metadata");
            assert_eq!(meta["resource"], audience);
            assert_eq!(meta["authorization_servers"], json!([issuer]));
            assert_eq!(wrong_aud_status, 401);
            assert_eq!(expired_status, 401);
            assert!(run_started, "run_start must succeed with valid OAuth token");
        },
    );

    run.finish()
        .expect("J-048's document is written")
        .assert_proved();
}
