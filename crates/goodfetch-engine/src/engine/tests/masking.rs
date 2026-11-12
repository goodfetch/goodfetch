//! Masking through the engine's public interface over the in-memory network
//! (spec #564's protocol seam; ADRs 0014, 0016 and 0092): the value of a
//! credential setting a Run resolved, here its key-encryption key named by
//! `run_state.key_reference`, is masked as `%run_state.key_reference%` in
//! every message a Client receives (call results, events and `$/log`
//! lines), in what run state and the Run index keep (its warnings and
//! progress message) and in the Run trace, raw and in every encoding. A
//! test operation stands for a page whose text reflects the value: it
//! logs that text, hands it over as a warning, writes it into the Run
//! trace and returns it. With `security.redaction` off, the value stays in
//! the Run trace alone.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::ffi::OsString;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::page::events::PAGE_REQUEST_FAILED;
use goodfetch_protocol::run::{RunId, RunKind, RunWarning, StartInput};
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use serde_json::{Value, json};
use tracing_subscriber::layer::SubscriberExt as _;

use super::{Client, attached_with, dependencies, engine_with};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::engine::Dependencies;
use crate::log::LogEvents;
use crate::run_trace::RequestLog;
use crate::runs::{EndedBy, Start, Started};

/// The variable the Run's key-encryption key is read from.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

/// A key-encryption key no canary holds, for the positive control.
const UNPLANTED_KEY: &str = "a key-encryption key no canary holds";

/// Logs, warns, traces and returns a text, as a page reflecting it would.
const REFLECT: Operation = Operation {
    name: "test.reflect",
    description: "Logs a text, hands it over as a warning, writes it into the Run trace and returns it.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

async fn reflect(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let text = params["text"].as_str().unwrap().to_owned();
    let run = context.runs().live(run_id).await.unwrap();
    tracing::info!(text = %text, "the page said {text}");
    run.warn(RunWarning {
        code: "page_said".to_owned(),
        message: text.clone(),
        sessions: Vec::new(),
        folder: None,
    })
    .await
    .unwrap();
    RequestLog::new(
        run.owner().child(),
        run.folder(),
        run.secrets().mask().clone(),
        run.metrics().counted_work().clone(),
    )
    .record(&PAGE_REQUEST_FAILED, &json!({ "url": text }))
    .await
    .unwrap();
    Ok(json!({ "text": text }))
}

/// A text holding `value` raw, URL-encoded with `%20` and with `+`, as the
/// password of a Basic `Authorization` header, in base64url and in hex;
/// its message carries it JSON-escaped.
fn reflecting(value: &str) -> String {
    let percent: String = value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!(
        "raw {value} | url {percent} | form {} | Authorization: Basic {} | b64url {} | hex {} {}",
        percent.replace("%20", "+"),
        data_encoding::BASE64.encode(format!("user:{value}").as_bytes()),
        data_encoding::BASE64URL.encode(value.as_bytes()),
        data_encoding::HEXLOWER.encode(value.as_bytes()),
        data_encoding::HEXUPPER.encode(value.as_bytes()),
    )
}

/// `env`'s dependencies, whose environment holds the Run's key, a canary,
/// which the user configuration names by reference.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Goodfetch home's user configuration itself"
)]
fn keyed(env: &TestEnv, key: &str) -> Dependencies {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        format!(
            "version = \"0.1\"\n\n[settings]\nrun_state.key_reference = \"env:{KEY_VARIABLE}\"\n"
        ),
    )
    .unwrap();
    Dependencies {
        environment: [(OsString::from(KEY_VARIABLE), OsString::from(key))].into(),
        ..dependencies(env)
    }
}

/// Every message the Client received while a Run of `settings` started,
/// reflected `text` and finished, its `hello` asking for `info` log lines.
fn reflected(env: &TestEnv, key: &str, settings: &Value, text: &str) -> Vec<Value> {
    let log_events = LogEvents::new();
    let subscriber = tracing_subscriber::registry().with(log_events.layer());
    let engine = engine_with(
        Product::Collie,
        Some(Contribution::new("test-reflect").with_handler(Handler::new(&REFLECT, reflect))),
    )
    .with_log_events(log_events);
    let calls = [
        (
            "run.start",
            json!({ "run_id": "masked", "settings": settings }),
        ),
        ("events.subscribe", json!({})),
        ("test.reflect", json!({ "run_id": "masked", "text": text })),
        (
            "run.set_progress_message",
            json!({ "run_id": "masked", "message": text }),
        ),
        ("run.finish", json!({ "run_id": "masked" })),
    ];
    tracing::subscriber::with_default(subscriber, || {
        attached_with(
            &engine,
            || keyed(env, key),
            |mut client: Client| async move {
                let mut received = Vec::new();
                client
                    .send(&[json!({
                        "jsonrpc": "2.0", "id": 0, "method": "hello",
                        "params": {
                            "protocol_version": "0.1", "surface": "sdk_typescript",
                            "needs": "collie", "product_version": "1.4.0", "log_level": "info",
                        },
                    })])
                    .await;
                for (id, (method, params)) in (1_i64..).zip(calls) {
                    client
                        .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
                        .await;
                    if method == "events.subscribe" {
                        client
                            .send(&[json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": id, "credits": 64 } })])
                            .await;
                        continue;
                    }
                    loop {
                        let message = client.next().await.expect("the engine replies");
                        let replied = message["id"] == json!(id);
                        received.push(message);
                        if replied {
                            break;
                        }
                    }
                }
                client.close().await;
                while let Some(message) = client.next().await {
                    received.push(message);
                }
                received
            },
        )
    })
}

/// Each leak of `canaries` in `received` and in every file of `env`'s
/// folder, as the canary and the output it is in.
fn leaks(canaries: &Canaries, env: &TestEnv, received: &[Value]) -> Vec<String> {
    let received = serde_json::to_vec(received).unwrap();
    let mut leaks: Vec<String> = canaries
        .leaks(&[
            Output::Bytes {
                name: "received",
                bytes: &received,
            },
            Output::Path(env.folder()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{:?} in {}", hit.encoding, relative(env, &hit.output)))
        .collect();
    leaks.sort();
    leaks.dedup();
    leaks
}

fn relative(env: &TestEnv, output: &str) -> String {
    output
        .strip_prefix(&env.folder().display().to_string())
        .unwrap_or(output)
        .to_owned()
}

/// Covers: story:4-56, setting:run_state.key_reference
#[test]
fn a_resolved_credential_is_masked_as_its_key_in_every_message_run_state_and_the_run_trace() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();

    let received = reflected(&env, &key, &json!({}), &reflecting(&key));

    let mut masked: Vec<&str> = received
        .iter()
        .filter(|message| {
            message
                .to_string()
                .contains("raw %run_state.key_reference% |")
        })
        .map(|message| {
            message["method"]
                .as_str()
                .or_else(|| message["result"].get("text").map(|_| "result"))
                .or_else(|| message["result"].get("warnings").map(|_| "report"))
                .or_else(|| message["result"].get("message").map(|_| "progress"))
                .unwrap_or("other")
        })
        .collect();
    masked.sort_unstable();
    masked.dedup();
    assert_eq!(
        (leaks(&canaries, &env, &received), masked),
        (
            Vec::<String>::new(),
            vec!["$/events", "$/log", "progress", "report", "result"]
        )
    );
}

/// Covers: story:4-76, setting:security.redaction
#[test]
fn with_redaction_off_a_resolved_credential_stays_only_in_the_run_trace() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();

    let received = reflected(
        &env,
        &key,
        &json!({ "security": { "redaction": false } }),
        &reflecting(&key),
    );

    let in_the_trace: Vec<String> = leaks(&canaries, &env, &received)
        .into_iter()
        .map(|leak| leak.split(" in ").nth(1).unwrap_or_default().to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let weakened: Vec<&Value> = received
        .iter()
        .filter_map(|message| message["result"]["weakened"].as_array())
        .flatten()
        .map(|weakened| &weakened["key"])
        .collect();
    assert_eq!(
        (in_the_trace, weakened),
        (
            vec!["/home/1/runs/masked/trace/requests.jsonl".to_owned()],
            vec![&json!("security.redaction")]
        )
    );
}

/// The positive control of the searches above (spec #564): the same Run
/// reflects the canary, but resolves another key-encryption key, so no
/// `Mask` knows the canary, and the same search finds it in every output
/// the Run wrote it to. A search that finds nothing then proves what it
/// claims, with no switch in the masking path. The run file's pages are in
/// its WAL, which its writer leaves beside it once the Run ended (#2117),
/// and so is the Run index's row, whose connections leave their WAL as it
/// is when they close (#2110).
/// Covers: story:4-91
#[test]
fn with_the_reflected_value_resolved_by_no_run_the_search_finds_it_in_every_output() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let reflected_value = canaries.plant(SecretKind::RunDataKey).value().to_owned();

    let received = reflected(
        &env,
        UNPLANTED_KEY,
        &json!({}),
        &reflecting(&reflected_value),
    );

    let outputs: std::collections::BTreeSet<String> = leaks(&canaries, &env, &received)
        .into_iter()
        .map(|leak| leak.split(" in ").nth(1).unwrap_or_default().to_owned())
        .collect();
    assert_eq!(
        outputs.into_iter().collect::<Vec<_>>(),
        vec![
            "/home/1/run-index.sqlite-wal",
            "/home/1/runs/masked/run.sqlite-wal",
            "/home/1/runs/masked/trace/requests.jsonl",
            "received",
        ]
    );
}

/// Spec #564's short-value rule through the engine: a resolved value of
/// seven characters, a short Secret canary, is left in what a call returns
/// and an event carries, where a page read may hold it by chance, and is
/// masked in everything the engine keeps: run state, the Run index and the
/// Run trace. The search reads the messages as page reads, so it finds no
/// leak there, and every kept file as an output.
/// Covers: story:4-91, setting:run_state.key_reference
#[test]
fn a_short_resolved_value_stays_in_a_read_and_is_masked_in_what_the_engine_keeps() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries
        .plant_short(SecretKind::TypedPassword)
        .value()
        .to_owned();

    let received = reflected(&env, &key, &json!({}), &reflecting(&key));

    let read = serde_json::to_vec(&received).unwrap();
    let leaks: Vec<String> = canaries
        .leaks(&[
            Output::Read {
                name: "received",
                bytes: &read,
            },
            Output::Path(env.folder()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{:?} in {}", hit.encoding, relative(&env, &hit.output)))
        .collect();
    let returned = received
        .iter()
        .find_map(|message| message["result"]["text"].as_str())
        .is_some_and(|text| text.starts_with(&format!("raw {key} |")));
    assert_eq!((leaks, returned), (Vec::<String>::new(), true));
}

/// Starts a Run of its own under `run_id`, tagged `tags` with the crawl
/// seeds `seeds` as its inputs, as a crawl's start would.
const START_TAGGED: Operation = Operation {
    name: "test.start_tagged",
    description: "Starts a Run with tags and seeds.",
    ..REFLECT
};

async fn start_tagged(params: Value, context: CallContext) -> Result<Value, CallError> {
    let started = context
        .runs()
        .start(Start {
            run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
            tags: serde_json::from_value(params["tags"].clone()).unwrap(),
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: [(StartInput::Seeds, params["seeds"].clone())].into(),
            ended_by: EndedBy::Finish,
            initialize: None,
            policy_version: None,
            weakened: Vec::new(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: None,
            timeout: None,
            settings: None,
        })
        .await;
    Ok(match started {
        Ok(Started::New(_)) => json!("new"),
        Ok(Started::Existing(_)) => json!("existing"),
        Err(error) => json!(error.to_string()),
    })
}

/// A Run's tags and inputs, here a crawl's seeds, pass through the `Mask`
/// before run state and the Run index keep them (spec #564): a value
/// another Run resolved, its key-encryption key, is in no file once a Run
/// tagged and seeded with it started, and the same start again still
/// answers that Run rather than a conflict.
/// Covers: story:4-56, setting:run_state.key_reference
#[test]
fn a_runs_tags_and_inputs_are_masked_in_what_the_engine_keeps() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let engine = engine_with(
        Product::Collie,
        Some(
            Contribution::new("test-tagged")
                .with_handler(Handler::new(&START_TAGGED, start_tagged)),
        ),
    );
    let tagged = json!({
        "run_id": "tagged",
        "tags": [format!("token {key}")],
        "seeds": [format!("https://shop.test/?token={key}")],
    });

    let answers = attached_with(
        &engine,
        || keyed(&env, &key),
        |mut client: Client| async move {
            let mut answers = Vec::new();
            client
                .send(&[json!({
                    "jsonrpc": "2.0", "id": 0, "method": "hello",
                    "params": {
                        "protocol_version": "0.1", "surface": "sdk_typescript",
                        "needs": "collie", "product_version": "1.4.0",
                    },
                })])
                .await;
            client.next().await;
            for (id, (method, params)) in (1_i64..).zip([
                ("run.start", json!({ "run_id": "masked" })),
                ("test.start_tagged", tagged.clone()),
                ("test.start_tagged", tagged),
            ]) {
                client
                    .send(&[
                        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
                    ])
                    .await;
                loop {
                    let message = client.next().await.expect("the engine replies");
                    if message["id"] == json!(id) {
                        answers.push(message["result"].clone());
                        break;
                    }
                }
            }
            answers
        },
    );

    assert_eq!(
        (leaks(&canaries, &env, &[]), &answers[1..]),
        (Vec::<String>::new(), &[json!("new"), json!("existing")][..]),
        "{answers:?}"
    );
}
