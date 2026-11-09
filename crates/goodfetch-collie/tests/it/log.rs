//! The logs and traces of Collie's executable (spec #213's Seam 1; ADRs
//! 0043, 0078, 0122 and 0132): the Attached engine's JSON log in the
//! Goodfetch home, written only when `log.output` is `file`, as test
//! support turns it on for ADR 0072's exercise audit, which reads `rpc`'s
//! dispatch records in it, and `stdout` refused there; `log.format` on
//! stderr; the `$/log`
//! events a Client asks for in `hello`; the spans an OTLP collector
//! receives once `otlp.traces_endpoint` names it; and no Secret canary in
//! any log, raw or in Goodfetch's encodings.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the engine's log files once it has exited, and run the OTLP collector stand-in on a thread"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the OTLP collector stand-in listens outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::sync::mpsc;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    Canaries, DISPATCH_TARGET, MESSAGE_DEADLINE, Output, SecretKind, TestEnv,
};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use rstest::rstest;
use serde_json::{Value, json};

use goodfetch_test_client::{Attached, hello};

use crate::COLLIE;

/// Every JSON line of the log files in the home's log folder.
fn log_lines(home: &Path) -> Vec<Value> {
    let mut files: Vec<_> = std::fs::read_dir(home.join("logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
        .iter()
        .flat_map(|file| {
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<Value>>()
        })
        .collect()
}

/// With `log.output: file`, as test support sets it, an Attached engine
/// writes the JSON lines ADR 0072's exercise audit reads into the test's
/// Goodfetch home.
///
/// Covers: row:RT-17, setting:log.output, setting:log.level
#[test]
fn with_log_output_file_the_engine_writes_its_dispatch_records_as_json_lines_into_the_home() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "sample.crawl", "params": {} }));
    let _ = engine.next();

    let _ = engine.finish();

    let dispatched: Vec<(Value, Value)> = log_lines(env.home())
        .into_iter()
        .filter(|line| line["target"] == DISPATCH_TARGET)
        .map(|line| {
            (
                line["fields"]["method"].clone(),
                line["fields"]["error_code"].clone(),
            )
        })
        .collect();
    assert_eq!(
        dispatched,
        vec![
            (json!("hello"), Value::Null),
            (json!("sample.crawl"), json!("not_in_product")),
        ]
    );
}

/// With `log.output: none`, the default of an Attached engine, it writes no
/// log file.
///
/// Covers: row:RT-17, setting:log.output
#[test]
fn with_log_output_none_the_engine_writes_no_log_file() {
    let env = TestEnv::new();
    env.user_configuration("log.output = \"none\"\n");
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let _ = engine.finish();

    assert!(!env.home().join("logs").exists());
}

/// `log.output: stdout` on an Attached engine, whose stdout is its
/// protocol, fails its start with `setting_not_applicable`, before any
/// message.
///
/// Covers: row:RT-17, setting:log.output, error:setting_not_applicable
#[test]
fn log_output_stdout_on_an_attached_engine_fails_with_setting_not_applicable() {
    let env = TestEnv::new();
    env.user_configuration("log.output = \"stdout\"\n");
    let engine = Attached::start(&env, COLLIE);

    let (sent, stderr) = engine.finish_with_stderr();

    assert_eq!(
        (sent, stderr.contains("setting_not_applicable")),
        (vec![], true),
        "{stderr}"
    );
}

/// Each line of `stderr` that reads as a JSON value, and how many do not.
fn json_lines(stderr: &str) -> (Vec<Value>, usize) {
    let mut values = Vec::new();
    let mut text = 0;
    for line in stderr.lines().filter(|line| !line.is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => values.push(value),
            Err(_) => text += 1,
        }
    }
    (values, text)
}

/// Covers: story:3-105, setting:log.format
#[rstest]
#[case::json(Some("json"), true)]
#[case::text(Some("text"), false)]
#[case::unset(None, false)]
fn an_attached_engine_writes_its_logs_on_stderr_in_the_format_log_format_names(
    #[case] format: Option<&str>,
    #[case] json_lines_only: bool,
) {
    let env = TestEnv::new();
    if let Some(format) = format {
        env.user_configuration(&format!("log.format = \"{format}\""));
    }
    let mut engine = Attached::start_logging(
        &env,
        COLLIE,
        MESSAGE_DEADLINE,
        &format!("info,{DISPATCH_TARGET}=trace"),
        |_| {},
    );
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {} }));
    let _ = engine.next();

    let (_, stderr) = engine.finish_with_stderr();

    let (values, text) = json_lines(&stderr);
    let dispatched: Vec<&Value> = values
        .iter()
        .filter(|line| line["target"] == DISPATCH_TARGET)
        .map(|line| &line["fields"]["method"])
        .collect();
    if json_lines_only {
        assert_eq!(
            (
                text,
                dispatched,
                values.iter().all(|line| line["timestamp"].is_string())
            ),
            (0, vec![&json!("hello"), &json!("engine.info")], true)
        );
    } else {
        assert_eq!(
            (
                values.len(),
                stderr.contains("request answered method=\"engine.info\"")
            ),
            (0, true)
        );
    }
}

/// Every message `engine` sends until the reply to the request `id`, that
/// reply included.
fn until_reply(engine: &mut Attached, id: i64) -> Vec<Value> {
    let mut read = Vec::new();
    while let Some(message) = engine.next() {
        let replied = message["id"] == json!(id);
        read.push(message);
        if replied {
            break;
        }
    }
    read
}

/// Covers: story:3-105, operation:connection.set_log_level
#[test]
fn a_client_whose_hello_names_a_log_level_receives_the_engines_log_events_as_log_notifications() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    let mut hello = hello(Product::Collie);
    hello["params"]["log_level"] = json!("trace");
    let mut sent = Vec::new();
    engine.send(&hello);
    sent.extend(until_reply(&mut engine, 0));
    for (id, request) in [
        (1, json!({ "method": "engine.info", "params": {} })),
        (
            2,
            json!({ "method": "connection.set_log_level", "params": { "log_level": "off" } }),
        ),
        (3, json!({ "method": "engine.info", "params": {} })),
    ] {
        let mut request = request;
        request["jsonrpc"] = json!("2.0");
        request["id"] = json!(id);
        engine.send(&request);
        sent.extend(until_reply(&mut engine, id));
    }

    let (rest, _) = engine.finish();

    sent.extend(rest);
    let logged: Vec<(Value, Value)> = sent
        .iter()
        .filter(|message| {
            message["method"] == "$/log" && message["params"]["target"] == DISPATCH_TARGET
        })
        .map(|message| {
            (
                message["params"]["level"].clone(),
                message["params"]["fields"]["method"].clone(),
            )
        })
        .collect();
    assert_eq!(logged, vec![(json!("trace"), json!("engine.info"))]);
}

/// The spans of one OTLP export: each one's name and string attributes.
type Exported = Vec<(String, Vec<(String, String)>)>;

/// The spans of an export.
fn spans_of(request: &ExportTraceServiceRequest) -> Exported {
    request
        .resource_spans
        .iter()
        .flat_map(|resource| &resource.scope_spans)
        .flat_map(|scope| &scope.spans)
        .map(|span| {
            let attributes = span
                .attributes
                .iter()
                .filter_map(
                    |attribute| match attribute.value.as_ref()?.value.as_ref()? {
                        AnyValue::StringValue(value) => {
                            Some((attribute.key.clone(), value.clone()))
                        }
                        _ => None,
                    },
                )
                .collect();
            (span.name.clone(), attributes)
        })
        .collect()
}

/// Serves one Connection of the collector stand-in: each request's body,
/// read as an `M`, answered `200` with nothing.
fn serve_exports<M: prost::Message + Default>(
    stream: std::net::TcpStream,
    exports: &mpsc::Sender<M>,
) {
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut length = 0;
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let _ = exports.send(M::decode(body.as_slice()).unwrap());
        let _ = writer.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\n\r\n",
        );
    }
}

/// An OTLP/HTTP collector stand-in on loopback, on a thread of its own: its
/// traces endpoint, and the spans of each export it receives.
fn collector() -> (String, mpsc::Receiver<ExportTraceServiceRequest>) {
    let (endpoint, received, _) = recorded_collector();
    (endpoint, received)
}

/// The collector stand-in of [`collector`], with its connection record:
/// one message for each connection it accepted.
fn recorded_collector() -> (
    String,
    mpsc::Receiver<ExportTraceServiceRequest>,
    mpsc::Receiver<()>,
) {
    collector_of("/v1/traces")
}

/// An OTLP/HTTP collector stand-in on loopback, on a thread of its own: its
/// endpoint at `path`, each export it receives, read as an `M`, and its
/// connection record, one message for each connection it accepted.
pub(crate) fn collector_of<M: prost::Message + Default + 'static>(
    path: &str,
) -> (String, mpsc::Receiver<M>, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}{path}", listener.local_addr().unwrap());
    let (exports, received) = mpsc::channel();
    let (accepted, connections) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let _ = accepted.send(());
            serve_exports(stream, &exports);
        }
    });
    (endpoint, received, connections)
}

/// Story 4-16 (ADR 0017): the OTLP collector is an Operator-configured
/// endpoint, reached on loopback by default; an Operator policy that sets
/// `egress.no_private_endpoints` refuses it before any connection, so the
/// collector's record shows none and it receives no span, while the run
/// itself goes on.
/// Covers: story:4-16, setting:egress.no_private_endpoints, setting:otlp.traces_endpoint
#[rstest]
#[case::by_default(false, 1)]
#[case::under_no_private_endpoints(true, 0)]
fn a_collector_on_loopback_is_refused_only_under_an_operator_policy_of_no_private_endpoints(
    #[case] refused: bool,
    #[case] runs: usize,
) {
    let env = TestEnv::new();
    let (endpoint, exports, connections) = recorded_collector();
    env.user_configuration(&format!("otlp.traces_endpoint = \"{endpoint}\""));
    if refused {
        let layout = HomeLayout::of(env.home());
        std::fs::create_dir_all(layout.per_major()).unwrap();
        std::fs::write(
            layout.operator_policy(),
            "version = \"0.1\"\n\n[settings.egress]\nno_private_endpoints = true\n",
        )
        .unwrap();
    }
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "run.start", "params": { "run_id": "traced" },
    }));
    let started = engine.next().unwrap();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 2, "method": "run.finish", "params": { "run_id": "traced" },
    }));
    let _ = engine.next();

    let (_, succeeded) = engine.finish();

    let exported = exports
        .try_iter()
        .flat_map(|request| spans_of(&request))
        .filter(|(name, _)| name == "run")
        .count();
    let accepted = connections.try_iter().count();
    assert_eq!(
        (succeeded, &started["error"], exported, accepted.min(1)),
        (true, &Value::Null, runs, runs)
    );
}

/// Covers: story:3-105, setting:otlp.traces_endpoint
#[test]
fn with_otlp_traces_endpoint_set_the_collector_receives_each_runs_span_with_its_run_id() {
    let env = TestEnv::new();
    let (endpoint, exports) = collector();
    env.user_configuration(&format!("otlp.traces_endpoint = \"{endpoint}\""));
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "run.start", "params": { "run_id": "traced" },
    }));
    let _ = engine.next();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 2, "method": "run.finish", "params": { "run_id": "traced" },
    }));
    let _ = engine.next();

    let (_, succeeded) = engine.finish();

    let runs: Exported = exports
        .try_iter()
        .flat_map(|request| spans_of(&request))
        .filter(|(name, _)| name == "run")
        .collect();
    assert_eq!(
        (succeeded, runs),
        (
            true,
            vec![(
                "run".to_owned(),
                vec![
                    ("target".to_owned(), "goodfetch_engine::runs".to_owned()),
                    ("run_id".to_owned(), "traced".to_owned())
                ]
            )]
        )
    );
}

/// The caller's span a Client's request names, in W3C Trace Context's
/// example `traceparent`.
const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

/// Covers: story:3-146, setting:otlp.traces_endpoint
#[test]
fn with_otlp_traces_endpoint_set_a_requests_server_span_continues_the_trace_its_meta_names() {
    let env = TestEnv::new();
    let (endpoint, exports) = collector();
    env.user_configuration(&format!("otlp.traces_endpoint = \"{endpoint}\""));
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "engine.info",
        "params": { "_meta": { "traceparent": TRACEPARENT } },
    }));
    let reply = engine.next().unwrap();

    let (_, succeeded) = engine.finish();

    let served: Vec<(Vec<u8>, Vec<u8>, i32)> = exports
        .try_iter()
        .flat_map(|request| request.resource_spans)
        .flat_map(|resource| resource.scope_spans)
        .flat_map(|scope| scope.spans)
        .filter(|span| span.name == "engine.info")
        .map(|span| (span.trace_id, span.parent_span_id, span.kind))
        .collect();
    assert!(reply.get("result").is_some(), "{reply}");
    assert_eq!(
        (succeeded, served),
        (
            true,
            vec![(
                vec![
                    0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c,
                    0x80, 0x31, 0x9c
                ],
                vec![0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31],
                // OTLP's `SPAN_KIND_SERVER`.
                2,
            )]
        )
    );
}

/// Each byte of `value` percent-encoded, as a URL may carry it.
fn percent_encoded(value: &str) -> String {
    value.bytes().fold(String::new(), |mut encoded, byte| {
        let _ = write!(encoded, "%{byte:02X}");
        encoded
    })
}

/// Secret canaries a Client writes where the engine records what it reads,
/// a URL's password, path and query, a method's name, a typed value and a
/// call's ID, reach no log line, raw or in any of Goodfetch's encodings
/// (ADR 0067), and the page's span records the URL's origin alone.
///
/// Covers: story:3-105
#[test]
fn no_secret_canary_a_client_sends_reaches_the_engines_logs() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let password = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let typed = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let url = format!(
        "https://user:{}@example.test/{}?token={}",
        percent_encoded(&password),
        percent_encoded(&token),
        percent_encoded(&token)
    );
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    for request in [
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "page.goto",
            "params": { "page_id": "page-1", "url": url },
        }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": format!("token.{token}"), "params": {} }),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "locator.fill",
            "params": { "locator": { "page_id": "page-1", "selector": "#password" }, "value": typed },
        }),
        json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": { "token": token } } }),
    ] {
        engine.send(&request);
    }

    let (_, stderr) = engine.finish_with_stderr();

    let leaks = canaries
        .leaks(&[
            Output::Path(&env.home().join("logs")),
            Output::Bytes {
                name: "stderr",
                bytes: stderr.as_bytes(),
            },
        ])
        .unwrap();
    let origins: Vec<Value> = log_lines(env.home())
        .into_iter()
        .filter(|line| line["span"]["name"] == "page" && line["span"]["operation"] == "page.goto")
        .map(|line| line["span"]["url.origin"].clone())
        .collect();
    assert_eq!(
        (leaks, origins.first().cloned()),
        (vec![], Some(json!("https://example.test")))
    );
}
