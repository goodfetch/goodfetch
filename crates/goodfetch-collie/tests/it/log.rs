//! The logs and traces of Collie's executable (spec #213's Seam 1; ADRs
//! 0043, 0078, 0122 and 0132): the Attached engine's JSON log in the
//! Goodfetch home, written only when `GOODFETCH_LOG_OUTPUT` is `file`, as
//! test support turns it on for ADR 0072's exercise audit, which reads
//! `rpc`'s dispatch records in it; `log.format` on stderr; the `$/log`
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

use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use prost::Message as _;
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};

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

#[test]
fn with_log_output_file_the_engine_writes_its_dispatch_records_as_json_lines_into_the_home() {
    let env = TestEnv::new();
    let mut engine = Attached::start_with(&env, |command| {
        command.env("GOODFETCH_LOG_OUTPUT", "file");
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "sample.crawl", "params": {} }));
    let _ = engine.next();

    let _ = engine.finish();

    let dispatched: Vec<(Value, Value)> = log_lines(env.home())
        .into_iter()
        .filter(|line| line["target"] == "goodfetch::rpc::dispatch")
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

#[test]
fn without_log_output_the_engine_writes_no_log_file() {
    let env = TestEnv::new();
    let mut engine = Attached::start_with(&env, |command| {
        command.env_remove("GOODFETCH_LOG_OUTPUT");
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    let _ = engine.finish();

    assert!(!env.home().join("logs").exists());
}

#[test]
fn an_unknown_log_output_fails_the_start() {
    let env = TestEnv::new();
    let engine = Attached::start_with(&env, |command| {
        command.env("GOODFETCH_LOG_OUTPUT", "stdout");
    });

    let (sent, succeeded) = engine.finish();

    assert_eq!((sent, succeeded), (vec![], false));
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
    let mut engine = Attached::start_with(&env, |command| {
        command.env("GOODFETCH_LOG", "info,goodfetch::rpc::dispatch=trace");
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {} }));
    let _ = engine.next();

    let (_, stderr) = engine.finish_with_stderr();

    let (values, text) = json_lines(&stderr);
    let dispatched: Vec<&Value> = values
        .iter()
        .filter(|line| line["target"] == "goodfetch::rpc::dispatch")
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

/// Covers: story:3-105, operation:connection.set_log_level
#[test]
fn a_client_whose_hello_names_a_log_level_receives_the_engines_log_events_as_log_notifications() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    let mut hello = hello("collie", "0.1");
    hello["params"]["log_level"] = json!("trace");
    engine.send(&hello);
    let _ = engine.next();
    engine.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {} }));
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 2, "method": "connection.set_log_level",
        "params": { "log_level": "off" },
    }));
    engine.send(&json!({ "jsonrpc": "2.0", "id": 3, "method": "engine.info", "params": {} }));

    let (sent, _) = engine.finish();

    let logged: Vec<(Value, Value)> = sent
        .iter()
        .filter(|message| {
            message["method"] == "$/log"
                && message["params"]["target"] == "goodfetch::rpc::dispatch"
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

/// The spans of an export's protobuf body.
fn spans_of(body: &[u8]) -> Exported {
    let request = ExportTraceServiceRequest::decode(body).unwrap();
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
/// answered `200` with nothing.
fn serve_exports(stream: std::net::TcpStream, exports: &mpsc::Sender<Exported>) {
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
        let _ = exports.send(spans_of(&body));
        let _ = writer.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\n\r\n",
        );
    }
}

/// An OTLP/HTTP collector stand-in on loopback, on a thread of its own: its
/// traces endpoint, and the spans of each export it receives.
fn collector() -> (String, mpsc::Receiver<Exported>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1/traces", listener.local_addr().unwrap());
    let (exports, received) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            serve_exports(stream, &exports);
        }
    });
    (endpoint, received)
}

/// Covers: story:3-105, setting:otlp.traces_endpoint
#[test]
fn with_otlp_traces_endpoint_set_the_collector_receives_each_runs_span_with_its_run_id() {
    let env = TestEnv::new();
    let (endpoint, exports) = collector();
    env.user_configuration(&format!("otlp.traces_endpoint = \"{endpoint}\""));
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
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
        .flatten()
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
    let mut engine = Attached::start_with(&env, |command| {
        command.env("GOODFETCH_LOG_OUTPUT", "file");
    });
    engine.send(&hello("collie", "0.1"));
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
