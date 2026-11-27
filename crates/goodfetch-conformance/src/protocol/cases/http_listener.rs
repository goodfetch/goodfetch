//! The Daemon's HTTP JSON-RPC listener's conformance cases (`POST /v1/rpc`,
//! spec #608 / #1400, ADRs 0027, 0030, 0067, 0099 and 0132): verifies `hello`
//! headers, outliving Run lifecycle, `400`/`401`/`403` refusals carrying the
//! JSON-RPC error, and trace-header fault tolerance against a Daemon started
//! with `http_listener.enabled = true` and `http_listener.port = 0`.
#![expect(
    clippy::disallowed_types,
    reason = "the conformance harness speaks HTTP over TCP to the Daemon's HTTP listener from outside the engine (ADR 0060's note for the harness)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness runs the executable's `daemon http-listener-token` command and speaks HTTP over TCP outside any async worker"
)]

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::process::Command;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{NOT_IN_PRODUCT, PROTOCOL_VERSION_UNSUPPORTED};
use goodfetch_protocol::daemon::STATUS;
use goodfetch_protocol::engine::ENGINE_INFO;
use goodfetch_protocol::http_listener::{
    EVENTS_PATH, HEADER_FORM, HEADER_PRODUCT, HEADER_PROTOCOL, RPC_PATH,
};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START};
use serde_json::{Value, json};

use super::Checks;
use crate::protocol::connection::{Connection, Gone, code_of};
use crate::protocol::target::{DEADLINE, EngineTarget};

/// A conformance case for the Daemon's HTTP JSON-RPC listener.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    StartOutlivingRun,
    RefusedHelloHeaders,
    TokenRefusals,
    HostAndOriginRefusals,
    MalformedTraceHeaders,
    EventsSseResume,
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "post-rpc-carries-hello-headers-and-starts-an-outliving-run" => Self::StartOutlivingRun,
            "post-rpc-refuses-bad-hello-headers-with-400-and-json-rpc-error" => {
                Self::RefusedHelloHeaders
            }
            "post-rpc-refuses-missing-wrong-or-query-token-with-401" => Self::TokenRefusals,
            "post-rpc-refuses-foreign-host-or-origin-with-403" => Self::HostAndOriginRefusals,
            "post-rpc-drops-malformed-or-over-long-trace-headers-without-refusing" => {
                Self::MalformedTraceHeaders
            }
            "get-events-streams-sse-and-resumes-from-last-event-id-and-after" => {
                Self::EventsSseResume
            }
            _ => return None,
        })
    }

    /// Runs the case against the target Daemon's HTTP listener.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        let Some(endpoint) = discover(connection, target, checks)? else {
            return Ok(());
        };
        match self {
            Self::StartOutlivingRun => start_outliving_run(&endpoint, checks),
            Self::RefusedHelloHeaders => refused_hello_headers(&endpoint, checks),
            Self::TokenRefusals => token_refusals(&endpoint, checks),
            Self::HostAndOriginRefusals => host_and_origin_refusals(&endpoint, checks),
            Self::MalformedTraceHeaders => malformed_trace_headers(&endpoint, checks),
            Self::EventsSseResume => events_sse_resume(&endpoint, checks),
        }
        Ok(())
    }
}

struct HttpEndpoint {
    address: SocketAddr,
    token: String,
    protocol: String,
}

/// Discovers the bound HTTP listener address from `daemon.status` and its token
/// from `<executable> daemon http-listener-token --json`.
fn discover(
    connection: &mut Connection,
    target: &EngineTarget,
    checks: &mut Checks,
) -> Result<Option<HttpEndpoint>, Gone> {
    let id = connection.request(STATUS.name, &json!({}))?;
    let status = connection.answer(id)?.message;
    let Some(address_text) = status["result"]["http_listener"].as_str() else {
        checks.check(
            "daemon.status reports the bound http_listener address",
            false,
        );
        return Ok(None);
    };
    let Some(address) = checks.needs(
        "daemon.status reports a valid SocketAddr for http_listener",
        address_text.parse::<SocketAddr>(),
    ) else {
        return Ok(None);
    };
    let output = Command::new(&target.executable)
        .env_clear()
        .envs(&target.environment)
        .current_dir(&target.folder)
        .args(["daemon", "http-listener-token", "--json"])
        .output();
    let Some(output) = checks.needs("`daemon http-listener-token --json` runs", output) else {
        return Ok(None);
    };
    if !output.status.success() {
        checks.check("`daemon http-listener-token --json` succeeds", false);
        return Ok(None);
    }
    let Some(parsed) = checks.needs(
        "`daemon http-listener-token --json` prints JSON",
        serde_json::from_slice::<Value>(&output.stdout),
    ) else {
        return Ok(None);
    };
    let Some(token) = parsed["token"].as_str() else {
        checks.check(
            "`daemon http-listener-token --json` returns a token string",
            false,
        );
        return Ok(None);
    };
    Ok(Some(HttpEndpoint {
        address,
        token: token.to_owned(),
        protocol: PROTOCOL_VERSION.to_string(),
    }))
}

/// Sends one HTTP/1.1 POST request and parses `(status_code, json_body)`.
fn send_post(
    address: SocketAddr,
    target: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<(u16, Value), String> {
    let mut stream =
        TcpStream::connect_timeout(&address, DEADLINE).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(DEADLINE))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(DEADLINE))
        .map_err(|error| error.to_string())?;
    let mut wire = String::new();
    let _ = write!(
        wire,
        "POST {target} HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
    {
        let _ = write!(wire, "Host: {address}\r\n");
    }
    for (name, value) in headers {
        let _ = write!(wire, "{name}: {value}\r\n");
    }
    wire.push_str("\r\n");
    wire.push_str(body);
    stream
        .write_all(wire.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8(response).map_err(|error| error.to_string())?;
    let (head, response_body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| "response had no HTTP header separator".to_owned())?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "response had no status line".to_owned())?
        .parse::<u16>()
        .map_err(|error| error.to_string())?;
    let parsed: Value = serde_json::from_str(response_body).map_err(|error| error.to_string())?;
    Ok((status, parsed))
}

fn rpc_body(id: i64, method: &str, params: &Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
    .to_string()
}

fn start_outliving_run(endpoint: &HttpEndpoint, checks: &mut Checks) {
    let auth = format!("Bearer {}", endpoint.token);
    let headers = [
        ("Authorization", auth.as_str()),
        (HEADER_PROTOCOL, endpoint.protocol.as_str()),
        (HEADER_PRODUCT, Product::Collie.as_str()),
        (HEADER_FORM, "catalog"),
    ];
    let Some((status, started)) = checks.needs(
        "POST /v1/rpc starts a Run",
        send_post(
            endpoint.address,
            RPC_PATH,
            &headers,
            &rpc_body(1, RUN_START.name, &json!({ "run_id": "conformance-http" })),
        ),
    ) else {
        return;
    };
    checks.check(
        "POST /v1/rpc run.start answers HTTP 200 with the requested run_id",
        status == 200 && started["result"]["run_id"] == json!("conformance-http"),
    );

    let Some((status, daemon_state)) = checks.needs(
        "POST /v1/rpc reads daemon.status",
        send_post(
            endpoint.address,
            RPC_PATH,
            &headers,
            &rpc_body(2, STATUS.name, &json!({})),
        ),
    ) else {
        return;
    };
    checks.check(
        "the Run started over POST /v1/rpc outlives its HTTP request",
        status == 200
            && daemon_state["result"]["live_runs"]
                .as_u64()
                .is_some_and(|count| count >= 1),
    );

    let Some((status, finished)) = checks.needs(
        "POST /v1/rpc finishes the Run",
        send_post(
            endpoint.address,
            RPC_PATH,
            &headers,
            &rpc_body(3, RUN_FINISH.name, &json!({ "run_id": "conformance-http" })),
        ),
    ) else {
        return;
    };
    checks.check(
        "POST /v1/rpc run.finish answers HTTP 200 with lifecycle finished",
        status == 200 && finished["result"]["lifecycle"] == json!("finished"),
    );
}

fn refused_hello_headers(endpoint: &HttpEndpoint, checks: &mut Checks) {
    let auth = format!("Bearer {}", endpoint.token);
    let body = rpc_body(1, ENGINE_INFO.name, &json!({}));

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with unsupported Goodfetch-Protocol answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[
                ("Authorization", auth.as_str()),
                (HEADER_PROTOCOL, "999.0"),
                (HEADER_PRODUCT, Product::Collie.as_str()),
                (HEADER_FORM, "catalog"),
            ],
            &body,
        ),
    ) {
        checks.check(
            "unsupported Goodfetch-Protocol gives HTTP 400 with protocol_version_unsupported",
            status == 400 && *code_of(&answer) == json!(PROTOCOL_VERSION_UNSUPPORTED.name),
        );
    }

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with another Product answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[
                ("Authorization", auth.as_str()),
                (HEADER_PROTOCOL, endpoint.protocol.as_str()),
                (HEADER_PRODUCT, Product::Foxhound.as_str()),
                (HEADER_FORM, "catalog"),
            ],
            &body,
        ),
    ) {
        checks.check(
            "Goodfetch-Product outside the target gives HTTP 400 with not_in_product",
            status == 400 && *code_of(&answer) == json!(NOT_IN_PRODUCT.name),
        );
    }

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with missing hello headers answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[("Authorization", auth.as_str())],
            &body,
        ),
    ) {
        checks.check(
            "missing Goodfetch-Protocol gives HTTP 400 carrying a JSON-RPC error",
            status == 400 && answer.get("error").is_some(),
        );
    }
}

fn token_refusals(endpoint: &HttpEndpoint, checks: &mut Checks) {
    let body = rpc_body(1, ENGINE_INFO.name, &json!({}));
    let base_headers = [
        (HEADER_PROTOCOL, endpoint.protocol.as_str()),
        (HEADER_PRODUCT, Product::Collie.as_str()),
        (HEADER_FORM, "catalog"),
    ];

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc without Authorization answers",
        send_post(endpoint.address, RPC_PATH, &base_headers, &body),
    ) {
        checks.check(
            "missing token gives HTTP 401 carrying a JSON-RPC error",
            status == 401 && answer.get("error").is_some(),
        );
    }

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with a wrong token answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[
                ("Authorization", "Bearer not-the-http-listener-token"),
                (HEADER_PROTOCOL, endpoint.protocol.as_str()),
                (HEADER_PRODUCT, Product::Collie.as_str()),
                (HEADER_FORM, "catalog"),
            ],
            &body,
        ),
    ) {
        checks.check(
            "wrong token gives HTTP 401 carrying a JSON-RPC error",
            status == 401 && answer.get("error").is_some(),
        );
    }

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with token in query string answers",
        send_post(
            endpoint.address,
            &format!("{RPC_PATH}?token={}", endpoint.token),
            &base_headers,
            &body,
        ),
    ) {
        checks.check(
            "token in query string gives HTTP 401 carrying a JSON-RPC error",
            status == 401 && answer.get("error").is_some(),
        );
    }
}

fn host_and_origin_refusals(endpoint: &HttpEndpoint, checks: &mut Checks) {
    let auth = format!("Bearer {}", endpoint.token);
    let body = rpc_body(1, ENGINE_INFO.name, &json!({}));

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with foreign Host answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[
                ("Host", "evil.example"),
                ("Authorization", auth.as_str()),
                (HEADER_PROTOCOL, endpoint.protocol.as_str()),
                (HEADER_PRODUCT, Product::Collie.as_str()),
                (HEADER_FORM, "catalog"),
            ],
            &body,
        ),
    ) {
        checks.check(
            "foreign Host gives HTTP 403 carrying a JSON-RPC error",
            status == 403 && answer.get("error").is_some(),
        );
    }

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with foreign Origin answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[
                ("Origin", "https://evil.example"),
                ("Authorization", auth.as_str()),
                (HEADER_PROTOCOL, endpoint.protocol.as_str()),
                (HEADER_PRODUCT, Product::Collie.as_str()),
                (HEADER_FORM, "catalog"),
            ],
            &body,
        ),
    ) {
        checks.check(
            "foreign Origin gives HTTP 403 carrying a JSON-RPC error",
            status == 403 && answer.get("error").is_some(),
        );
    }
}

fn malformed_trace_headers(endpoint: &HttpEndpoint, checks: &mut Checks) {
    let auth = format!("Bearer {}", endpoint.token);
    let body = rpc_body(1, ENGINE_INFO.name, &json!({}));
    let long_tracestate = format!("vendor={}", "a".repeat(2048));

    if let Some((status, answer)) = checks.needs(
        "POST /v1/rpc with malformed and over-long trace headers answers",
        send_post(
            endpoint.address,
            RPC_PATH,
            &[
                ("Authorization", auth.as_str()),
                (HEADER_PROTOCOL, endpoint.protocol.as_str()),
                (HEADER_PRODUCT, Product::Collie.as_str()),
                (HEADER_FORM, "catalog"),
                ("traceparent", "not-a-valid-traceparent"),
                ("tracestate", long_tracestate.as_str()),
            ],
            &body,
        ),
    ) {
        checks.check(
            "malformed or over-long trace headers are dropped and the RPC still answers HTTP 200",
            status == 200 && answer["result"]["product"] == json!(Product::Collie.as_str()),
        );
    }
}

#[derive(Debug, PartialEq)]
struct SseEvent {
    id: u64,
    event: String,
    data: Value,
}

/// Reads the HTTP status and first complete SSE event from `GET /v1/events`.
fn read_first_sse(
    address: SocketAddr,
    target: &str,
    headers: &[(&str, &str)],
) -> Result<(u16, Option<SseEvent>), String> {
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(DEADLINE))
        .build()
        .new_agent();
    let mut request = agent.get(format!("http://{address}{target}"));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.call().map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    if status != 200 {
        return Ok((status, None));
    }
    let mut reader = BufReader::new(response.into_body().into_reader());
    let (mut id, mut event, mut data) = (None, None, None);
    let mut line = String::new();
    while reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        > 0
    {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() && id.is_some() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("id: ") {
            id = v.parse::<u64>().ok();
        } else if let Some(v) = trimmed.strip_prefix("event: ") {
            event = Some(v.to_owned());
        } else if let Some(v) = trimmed.strip_prefix("data: ") {
            data = serde_json::from_str::<Value>(v).ok();
        }
        line.clear();
    }
    match (id, event, data) {
        (Some(id), Some(event), Some(data)) => Ok((status, Some(SseEvent { id, event, data }))),
        _ => Err("SSE stream ended before yielding a complete event".to_owned()),
    }
}

fn events_sse_resume(endpoint: &HttpEndpoint, checks: &mut Checks) {
    let auth = format!("Bearer {}", endpoint.token);
    let rpc_headers = [
        ("Authorization", auth.as_str()),
        (HEADER_PROTOCOL, endpoint.protocol.as_str()),
        (HEADER_PRODUCT, Product::Collie.as_str()),
        (HEADER_FORM, "catalog"),
    ];
    let Some((start_status, _)) = checks.needs(
        "POST /v1/rpc starts Run for SSE conformance",
        send_post(
            endpoint.address,
            RPC_PATH,
            &rpc_headers,
            &rpc_body(1, RUN_START.name, &json!({ "run_id": "conformance-sse" })),
        ),
    ) else {
        return;
    };
    checks.check("run.start answers HTTP 200", start_status == 200);

    let Some((status1, first)) = checks.needs(
        "GET /v1/events streams first Run event",
        read_first_sse(
            endpoint.address,
            &format!("{EVENTS_PATH}?run=conformance-sse&after=0"),
            &[("Authorization", auth.as_str())],
        ),
    ) else {
        return;
    };
    let Some(first) = first else {
        checks.check("GET /v1/events answers HTTP 200 with an SSE event", false);
        return;
    };
    checks.check(
        "first SSE event carries position id and run.axes_changed for conformance-sse",
        status1 == 200
            && first.event == "run.axes_changed"
            && first.data["data"]["run_id"] == json!("conformance-sse")
            && first.data["data"]["lifecycle"] == json!("running"),
    );

    let _ = checks.needs(
        "POST /v1/rpc finishes Run for SSE resume",
        send_post(
            endpoint.address,
            RPC_PATH,
            &rpc_headers,
            &rpc_body(2, RUN_FINISH.name, &json!({ "run_id": "conformance-sse" })),
        ),
    );

    let last_id = first.id.to_string();
    let by_header = checks.needs(
        "GET /v1/events reconnects with Last-Event-ID",
        read_first_sse(
            endpoint.address,
            &format!("{EVENTS_PATH}?run=conformance-sse"),
            &[
                ("Authorization", auth.as_str()),
                ("Last-Event-ID", last_id.as_str()),
            ],
        ),
    );
    let by_after = checks.needs(
        "GET /v1/events resumes with after=<position>",
        read_first_sse(
            endpoint.address,
            &format!("{EVENTS_PATH}?run=conformance-sse&after={last_id}"),
            &[("Authorization", auth.as_str())],
        ),
    );
    if let (Some((sh, Some(eh))), Some((sa, Some(ea)))) = (by_header, by_after) {
        checks.check(
            "Last-Event-ID and after produce the identical resumed SSE event",
            sh == 200
                && sa == 200
                && eh == ea
                && eh.id > first.id
                && eh.data["data"]["lifecycle"] == json!("finished"),
        );
    }

    if let Some((q_status, _)) = checks.needs(
        "GET /v1/events refuses query-string token",
        read_first_sse(
            endpoint.address,
            &format!("{EVENTS_PATH}?run=conformance-sse&token={}", endpoint.token),
            &[],
        ),
    ) {
        checks.check(
            "GET /v1/events refuses token in query string with HTTP 401",
            q_status == 401,
        );
    }
}
