//! The Daemon's HTTP JSON-RPC listener (`POST /v1/rpc`, spec #608 / #1400;
//! ADRs 0027, 0030, 0049, 0067 and 0099) through Collie's executable: off by
//! default, loopback on port 0 when configured for tests, token rotation via
//! `daemon rotate-token`, TLS from the Exposed Daemon's keys, and every
//! applicable row of the endpoint table (#375).
#![expect(
    clippy::disallowed_types,
    reason = "a test sends HTTP/TLS requests directly to the Daemon's HTTP listener from outside the engine (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write temporary certificate files and read the Daemon's token files directly"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::home::DaemonFile;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    AuditUnreadable, Canaries, Endpoint, EndpointRunner, Listener, ListenerKind, ListenerTls, Rule,
    RuntimeFile, TestEnv,
};
use serde_json::{Value, json};

use crate::daemon::{StopsTheDaemon, json_of, runtime_file, short_idle};
use crate::daemon_client;
use crate::daemon_run::{ANY_PORT, Foreground, token};
use crate::endpoint::provision_ops_token;
use crate::tls::SERVER_NAME;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Sends one raw HTTP/1.1 request to `address` and returns `(status_code, body_json)`.
fn raw_http_rpc(
    address: SocketAddr,
    target: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, Value) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut request = format!(
        "POST {target} HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    let mut has_host = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("host") {
            has_host = true;
        }
        let _ = write!(request, "{name}: {value}\r\n");
    }
    if !has_host {
        let _ = write!(request, "Host: {address}\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8(raw).unwrap();
    let (head, response_body) = text.split_once("\r\n\r\n").unwrap();
    let status: u16 = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let parsed: Value = serde_json::from_str(response_body).unwrap();
    (status, parsed)
}

fn post_rpc(address: SocketAddr, token: &str, body: &Value) -> (u16, Value) {
    let protocol = PROTOCOL_VERSION.to_string();
    let auth = format!("Bearer {token}");
    raw_http_rpc(
        address,
        "/v1/rpc",
        &[
            ("Authorization", &auth),
            ("Goodfetch-Protocol", &protocol),
            ("Goodfetch-Product", Product::Collie.as_str()),
            ("Goodfetch-Form", "catalog"),
        ],
        &body.to_string(),
    )
}

/// The endpoint table's audit reason each rule logs on `http_listener`.
fn reason(rule: Rule) -> Option<&'static str> {
    Some(match rule {
        Rule::TokenMissing | Rule::TokenInQueryString | Rule::OriginMissingWithoutHeaderToken => {
            "token_missing"
        }
        Rule::TokenWrong
        | Rule::TokenFromAnotherListener
        | Rule::TokenExpired
        | Rule::TokenRevoked
        | Rule::TokenScopedElsewhere => "token_wrong",
        Rule::HostForeign => "host_foreign",
        Rule::OriginForeign => "origin_foreign",
        Rule::PlainWhereTlsConfigured => "tls_required",
        Rule::OriginMissing | Rule::HelloNotFirst | Rule::ProbesAnswerStatusOnly => return None,
    })
}

/// Queries `daemon.status` over the machine Daemon's socket and returns the
/// bound HTTP listener address (`None` while `http_listener.enabled` is off).
fn daemon_http_listener(env: &TestEnv) -> Option<SocketAddr> {
    let found = RuntimeFile::read(&runtime_file(env)).unwrap();
    let mut client = daemon_client(&found);
    let status = client.call(1, "daemon.status", &json!({}));
    status["result"]["http_listener"]
        .as_str()
        .map(|text| text.parse().unwrap())
}

/// With default configuration, the HTTP listener is off and `daemon.status`
/// reports no HTTP listener address.
///
/// Covers: story:11-258, setting:http_listener.enabled
#[test]
fn the_http_listener_is_off_by_default() {
    let env = TestEnv::new();
    short_idle(&env, 60_000);
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);

    assert_eq!(daemon_http_listener(&env), None);
}

/// Enabled on port 0, a plain HTTP client starts an outliving Run via
/// `POST /v1/rpc`, verifies refusals (`401`, `403`, `400`), and `daemon
/// rotate-token` invalidates the old token.
///
/// Covers: story:11-258, story:11-259, story:11-261, command:daemon.http-listener-token, command:daemon.rotate-token
#[test]
fn plain_http_client_starts_a_run_and_rotates_the_http_listener_token() {
    let env = TestEnv::new();
    env.user_configuration(
        "daemon.idle_timeout = 60000\nhttp_listener.enabled = true\nhttp_listener.port = 0\n",
    );
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);

    let address = daemon_http_listener(&env).expect("http_listener address in daemon.status");
    assert!(address.ip().is_loopback() && address.port() != 0);

    let first_token = json_of(
        COLLIE.as_ref(),
        &env,
        &["daemon", "http-listener-token", "--json"],
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // 1. Plain HTTP client starts an outliving Run and finishes it on a later request.
    let (status_code, started) = post_rpc(
        address,
        &first_token,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "run.start",
            "params": { "run_id": "via-http-listener" }
        }),
    );
    assert_eq!(status_code, 200, "{started}");
    assert_eq!(started["result"]["run_id"], json!("via-http-listener"));

    let (status_code, daemon_state) = post_rpc(
        address,
        &first_token,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "daemon.status", "params": {} }),
    );
    assert_eq!(status_code, 200, "{daemon_state}");
    assert!(
        daemon_state["result"]["live_runs"]
            .as_u64()
            .is_some_and(|live| live >= 1),
        "Run started over POST /v1/rpc must outlive the HTTP request: {daemon_state}"
    );

    let (status_code, finished) = post_rpc(
        address,
        &first_token,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "run.finish",
            "params": { "run_id": "via-http-listener" }
        }),
    );
    assert_eq!(status_code, 200, "{finished}");
    assert_eq!(finished["result"]["lifecycle"], json!("finished"));

    // 2. Refusals: 401 (missing/wrong/query-string), 403 (foreign Host/Origin), 400 (hello).
    assert_http_refusals(address, &first_token);

    // 3. `daemon rotate-token` rotates the token and stops the running Daemon so
    // the restarted Daemon refuses the old token with 401 and accepts the new one.
    json_of(COLLIE.as_ref(), &env, &["daemon", "rotate-token", "--json"]);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let address_after = daemon_http_listener(&env).expect("http_listener address after restart");
    let rotated_token = json_of(
        COLLIE.as_ref(),
        &env,
        &["daemon", "http-listener-token", "--json"],
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(rotated_token, first_token);

    let (code_old, _) = post_rpc(
        address_after,
        &first_token,
        &json!({ "jsonrpc": "2.0", "id": 5, "method": "engine.info", "params": {} }),
    );
    let (code_new, info) = post_rpc(
        address_after,
        &rotated_token,
        &json!({ "jsonrpc": "2.0", "id": 6, "method": "engine.info", "params": {} }),
    );
    assert_eq!(
        (code_old, code_new, info["result"]["product"].as_str()),
        (401, 200, Some("collie"))
    );
}

fn assert_http_refusals(address: SocketAddr, token: &str) {
    let protocol = PROTOCOL_VERSION.to_string();
    let info_body = r#"{"jsonrpc":"2.0","id":4,"method":"engine.info","params":{}}"#;
    let (code_missing, _) = raw_http_rpc(
        address,
        "/v1/rpc",
        &[
            ("Goodfetch-Protocol", &protocol),
            ("Goodfetch-Product", "collie"),
            ("Goodfetch-Form", "catalog"),
        ],
        info_body,
    );
    let (code_query, _) = raw_http_rpc(
        address,
        &format!("/v1/rpc?token={token}"),
        &[
            ("Goodfetch-Protocol", &protocol),
            ("Goodfetch-Product", "collie"),
            ("Goodfetch-Form", "catalog"),
        ],
        info_body,
    );
    let auth = format!("Bearer {token}");
    let (code_host, _) = raw_http_rpc(
        address,
        "/v1/rpc",
        &[
            ("Host", "evil.example"),
            ("Authorization", &auth),
            ("Goodfetch-Protocol", &protocol),
            ("Goodfetch-Product", "collie"),
            ("Goodfetch-Form", "catalog"),
        ],
        info_body,
    );
    let (code_origin, _) = raw_http_rpc(
        address,
        "/v1/rpc",
        &[
            ("Origin", "https://evil.example"),
            ("Authorization", &auth),
            ("Goodfetch-Protocol", &protocol),
            ("Goodfetch-Product", "collie"),
            ("Goodfetch-Form", "catalog"),
        ],
        info_body,
    );
    let (code_proto, proto_body) = raw_http_rpc(
        address,
        "/v1/rpc",
        &[
            ("Authorization", &auth),
            ("Goodfetch-Protocol", "999.0"),
            ("Goodfetch-Product", "collie"),
            ("Goodfetch-Form", "catalog"),
        ],
        info_body,
    );
    let (code_product, product_body) = raw_http_rpc(
        address,
        "/v1/rpc",
        &[
            ("Authorization", &auth),
            ("Goodfetch-Protocol", &protocol),
            ("Goodfetch-Product", "foxhound"),
            ("Goodfetch-Form", "catalog"),
        ],
        info_body,
    );
    assert_eq!(
        (
            code_missing,
            code_query,
            code_host,
            code_origin,
            code_proto,
            proto_body["error"]["data"]["code"].as_str(),
            code_product,
            product_body["error"]["data"]["code"].as_str(),
        ),
        (
            401,
            401,
            403,
            403,
            400,
            Some("protocol_version_unsupported"),
            400,
            Some("not_in_product"),
        )
    );
}

/// With TLS configured via the Exposed Daemon's keys (`daemon.tls.*`), the
/// HTTP listener terminates TLS and passes every row of the endpoint table
/// (#375), logging each refusal with its listener and reason.
///
/// Covers: story:11-258, story:11-261
#[test]
fn the_http_listener_terminates_exposed_daemon_tls_and_passes_the_endpoint_table() {
    let env = TestEnv::new();
    let cert_file = env.folder().join("daemon.crt");
    let key_file = env.folder().join("daemon.key");
    let key_pair = rcgen::KeyPair::generate().unwrap();
    let generated = rcgen::CertificateParams::new(vec![SERVER_NAME.to_owned()])
        .unwrap()
        .self_signed(&key_pair)
        .unwrap();
    std::fs::write(&cert_file, generated.pem()).unwrap();
    std::fs::write(&key_file, key_pair.serialize_pem()).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let ops_token = provision_ops_token(&env, &mut canaries);
    env.user_configuration(&format!(
        "daemon.address = \"{ANY_PORT}\"\n\
         daemon.tls.cert_file = \"{}\"\n\
         daemon.tls.key_file = \"{}\"\n\
         http_listener.enabled = true\n\
         http_listener.port = 0\n",
        cert_file.display(),
        key_file.display(),
    ));

    let mut daemon = Foreground::start(&env, COLLIE, &[]);
    let http_served = daemon.record("the HTTP listener serves");
    let http_address: SocketAddr = http_served["address"].as_str().unwrap().parse().unwrap();
    daemon.serving();

    let daemon_token = token(&env);
    let http_token = std::fs::read_to_string(
        HomeLayout::of(env.home()).daemon_file(DaemonFile::HttpListenerToken),
    )
    .unwrap();
    let tls = ListenerTls::new(SERVER_NAME, generated.der().to_vec());
    let listener = Listener::new(
        ListenerKind::HttpListener,
        Endpoint::HttpRpc {
            address: http_address,
            protocol: PROTOCOL_VERSION.to_string(),
            product: Product::Collie.as_str().to_owned(),
        },
        http_token,
    )
    .with_other_listener_token(daemon_token)
    .with_scoped_elsewhere_token(ops_token)
    .with_revoked_token("revoked-http-listener-token")
    .over_tls(tls);

    let verifier = |kind: ListenerKind, rule: Rule| -> Result<bool, AuditUnreadable> {
        let Some(expected_reason) = reason(rule) else {
            return Ok(false);
        };
        let refusal = daemon.refusal();
        Ok(refusal.is_some_and(|refusal| {
            kind == ListenerKind::HttpListener
                && refusal["listener"] == json!("http_listener")
                && refusal["reason"] == json!(expected_reason)
        }))
    };

    let ran = EndpointRunner::new(verifier).run(&listener);
    let exited = daemon.terminate();

    assert_eq!((ran, exited), (Ok(()), true));
}

/// Reads the first complete SSE event `(id, event, data)` from `url` via `ureq`.
fn first_sse_event(url: &str, token: &str, last_event_id: Option<&str>) -> (u64, String, Value) {
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    let mut request = agent
        .get(url)
        .header("Authorization", &format!("Bearer {token}"));
    if let Some(id) = last_event_id {
        request = request.header("Last-Event-ID", id);
    }
    let response = request.call().unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let mut reader = BufReader::new(response.into_body().into_reader());
    let (mut id, mut event, mut data) = (None, None, None);
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap() > 0 {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() && id.is_some() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("id: ") {
            id = Some(v.parse::<u64>().unwrap());
        } else if let Some(v) = trimmed.strip_prefix("event: ") {
            event = Some(v.to_owned());
        } else if let Some(v) = trimmed.strip_prefix("data: ") {
            data = Some(serde_json::from_str::<Value>(v).unwrap());
        }
        line.clear();
    }
    (id.unwrap(), event.unwrap(), data.unwrap())
}

/// `GET /v1/events?run=<id>&after=<position>` streams a Run's events as SSE,
/// resumes identically via `Last-Event-ID` and `after` after dropping, and
/// refuses a token in the query string.
///
/// Covers: story:11-260
#[test]
fn get_events_streams_and_resumes_from_last_event_id_and_after() {
    let env = TestEnv::new();
    env.user_configuration(
        "daemon.idle_timeout = 60000\nhttp_listener.enabled = true\nhttp_listener.port = 0\n",
    );
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    json_of(COLLIE.as_ref(), &env, &["daemon", "start", "--json"]);
    let address = daemon_http_listener(&env).expect("http_listener bound");
    let token = json_of(
        COLLIE.as_ref(),
        &env,
        &["daemon", "http-listener-token", "--json"],
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let (start_code, _) = post_rpc(
        address,
        &token,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "run.start", "params": { "run_id": "sse-it" } }),
    );
    assert_eq!(start_code, 200);

    let first = first_sse_event(
        &format!("http://{address}/v1/events?run=sse-it&after=0"),
        &token,
        None,
    );
    assert_eq!(first.1, "run.axes_changed");
    assert_eq!(first.2["data"]["lifecycle"], json!("running"));

    let (finish_code, _) = post_rpc(
        address,
        &token,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "run.finish", "params": { "run_id": "sse-it" } }),
    );
    assert_eq!(finish_code, 200);

    let last_id = first.0.to_string();
    let by_header = first_sse_event(
        &format!("http://{address}/v1/events?run=sse-it"),
        &token,
        Some(&last_id),
    );
    let by_after = first_sse_event(
        &format!("http://{address}/v1/events?run=sse-it&after={last_id}"),
        &token,
        None,
    );
    assert_eq!(by_header, by_after);
    assert!(by_header.0 > first.0);
    assert_eq!(by_header.2["data"]["lifecycle"], json!("finished"));

    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let query_refused = agent
        .get(&format!(
            "http://{address}/v1/events?run=sse-it&token={token}"
        ))
        .call()
        .unwrap();
    assert_eq!(query_refused.status().as_u16(), 401);
}
