//! `Recordings` (ADR 0065): Model recordings replayed by `httpmock` at a
//! base URL, matched on method, path and the canonical JSON body; a request
//! with no recording fails, and so does a recording left unused. Each test
//! writes its recordings into its own folder, never into a folder named
//! `recordings/`, which the Proving ground's shape test reads as real ones.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use goodfetch_test_support::{
    ExchangeName, Recordings, RecordingsError, ServedExchanges, TestEnv, Unfinished,
    UnmatchedRequest,
};
use serde_json::{Value, json};

/// The committed schema of a recording file, which `xtask codegen` writes.
const RECORDING_SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/schemas/model-recording.schema.json"
);

const CHAT: &str = "/v1/chat/completions";

/// A folder of the test's own holding `file`, a recording of `exchanges`.
fn recording(env: &TestEnv, file: &str, exchanges: &[Value]) -> PathBuf {
    let folder = env.folder().join(env.unique_name("model-exchanges"));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join(file),
        json!({ "exchanges": exchanges }).to_string(),
    )
    .unwrap();
    folder
}

fn chat_request(content: &str) -> Value {
    json!({
        "model": "gpt-test",
        "messages": [{ "role": "user", "content": content }],
        "temperature": 0
    })
}

fn chat_exchange(content: &str, answer: &str) -> Value {
    json!({
        "request": { "method": "POST", "path": CHAT, "body": chat_request(content) },
        "response": {
            "status": 200,
            "headers": { "content-type": "application/json" },
            "body": { "choices": [{ "message": { "role": "assistant", "content": answer } }] }
        }
    })
}

/// An answer as a client reads it: its status, `content-type` and
/// `retry-after` headers, and body.
#[derive(Debug, PartialEq)]
struct Answer {
    status: u16,
    content_type: Option<String>,
    retry_after: Option<String>,
    body: String,
}

fn post(base_url: &str, path: &str, body: &str) -> Answer {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent
        .post(format!("{base_url}{path}"))
        .header("content-type", "application/json")
        .send(body)
        .unwrap();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().unwrap().to_owned())
    };
    let (content_type, retry_after) = (header("content-type"), header("retry-after"));
    Answer {
        status: response.status().as_u16(),
        content_type,
        retry_after,
        body: response.body_mut().read_to_string().unwrap(),
    }
}

fn name(file: &str, index: usize) -> ExchangeName {
    ExchangeName {
        file: file.to_owned(),
        index,
    }
}

#[test]
fn a_request_matching_a_recording_gets_the_recorded_response() {
    let env = TestEnv::new();
    let exchange = chat_exchange("hello", "hi there");
    let recordings = Recordings::serve(&recording(
        &env,
        "chat.json",
        std::slice::from_ref(&exchange),
    ))
    .unwrap();

    // The same JSON with its keys in another order and `0.0` for `0`, which
    // canonical JSON writes alike.
    let answer = post(
        &recordings.base_url(),
        CHAT,
        r#"{"temperature":0.0,"messages":[{"content":"hello","role":"user"}],"model":"gpt-test"}"#,
    );

    assert_eq!(answer.status, 200);
    assert_eq!(answer.content_type.as_deref(), Some("application/json"));
    assert_eq!(
        serde_json::from_str::<Value>(&answer.body).unwrap(),
        exchange["response"]["body"]
    );
    assert_eq!(
        recordings.finish(),
        Ok(ServedExchanges {
            recorded: vec![name("chat.json", 0)],
            synthetic: vec![],
        })
    );
}

#[test]
fn a_request_with_another_body_gets_no_recording_and_fails_the_test() {
    let env = TestEnv::new();
    let recordings = Recordings::serve(&recording(
        &env,
        "chat.json",
        &[chat_exchange("hello", "hi")],
    ))
    .unwrap();
    let changed = chat_request("goodbye").to_string();

    let answer = post(&recordings.base_url(), CHAT, &changed);

    assert_eq!(answer.status, 404);
    assert_eq!(
        recordings.finish(),
        Err(Unfinished {
            unused: vec![name("chat.json", 0)],
            unmatched: vec![UnmatchedRequest {
                method: "POST".to_owned(),
                path: CHAT.to_owned(),
                body: changed,
            }],
        })
    );
}

#[test]
fn a_recording_left_unused_fails_at_teardown() {
    let env = TestEnv::new();
    let recordings = Recordings::serve(&recording(
        &env,
        "chat.json",
        &[chat_exchange("hello", "hi")],
    ))
    .unwrap();

    let teardown = catch_unwind(AssertUnwindSafe(move || drop(recordings)));

    let message = *teardown.unwrap_err().downcast::<String>().unwrap();
    let expected = Unfinished {
        unused: vec![name("chat.json", 0)],
        unmatched: vec![],
    };
    assert_eq!(message, expected.to_string());
}

#[test]
fn two_exchanges_of_one_request_answer_in_turn() {
    let env = TestEnv::new();
    let recordings = Recordings::serve(&recording(
        &env,
        "retry.json",
        &[
            chat_exchange("hello", "first"),
            chat_exchange("hello", "second"),
        ],
    ))
    .unwrap();
    let request = chat_request("hello").to_string();

    let answers: Vec<Value> = (0..2)
        .map(|_| serde_json::from_str(&post(&recordings.base_url(), CHAT, &request).body).unwrap())
        .collect();

    assert_eq!(answers[0]["choices"][0]["message"]["content"], "first");
    assert_eq!(answers[1]["choices"][0]["message"]["content"], "second");
    assert!(recordings.finish().is_ok());
}

/// An exchange whose response is a stream of server-sent events, which a
/// recording keeps as the chunks the provider sent.
fn stream_exchange() -> (Value, String) {
    let chunks: Vec<String> = (0..12)
        .map(|n| format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"word {n} \"}}}}]}}\n\n"))
        .chain(["data: [DONE]\n\n".to_owned()])
        .collect();
    let exchange = json!({
        "request": { "method": "POST", "path": CHAT, "body": chat_request("stream") },
        "response": {
            "status": 200,
            "headers": { "content-type": "text/event-stream" },
            "chunks": chunks
        }
    });
    (exchange, chunks.concat())
}

#[test]
fn a_stream_replays_whole() {
    let env = TestEnv::new();
    let (exchange, whole) = stream_exchange();
    let recordings = Recordings::serve(&recording(&env, "stream.json", &[exchange])).unwrap();

    let answer = post(
        &recordings.base_url(),
        CHAT,
        &chat_request("stream").to_string(),
    );

    assert_eq!(answer.content_type.as_deref(), Some("text/event-stream"));
    assert_eq!(answer.body, whole);
    assert_eq!(recordings.pieces(), Vec::<Vec<usize>>::new());
}

/// Replays the stream through a re-cut of `seed`, and returns the body the
/// client read and the pieces the response was written in.
fn replay_cut(env: &TestEnv, seed: u64) -> (String, Vec<Vec<usize>>) {
    let (exchange, _) = stream_exchange();
    let recordings =
        Recordings::serve_recut(&recording(env, "stream.json", &[exchange]), seed).unwrap();
    let answer = post(
        &recordings.base_url(),
        CHAT,
        &chat_request("stream").to_string(),
    );
    let pieces = recordings.pieces();
    recordings.finish().unwrap();
    (answer.body, pieces)
}

#[test]
fn a_stream_re_cut_at_seeded_boundaries_reads_back_whole_and_cuts_alike_for_one_seed() {
    let env = TestEnv::new();
    let (_, whole) = stream_exchange();

    let (first_body, first_pieces) = replay_cut(&env, env.seed());
    let (second_body, second_pieces) = replay_cut(&env, env.seed());

    assert_eq!(first_body, whole);
    assert_eq!(second_body, whole);
    assert_eq!(first_pieces.len(), 1, "one response");
    assert!(first_pieces[0].len() > 1, "{first_pieces:?}");
    assert_eq!(first_pieces, second_pieces);
}

#[test]
fn another_seed_cuts_a_stream_elsewhere() {
    let env = TestEnv::new();

    let (_, first) = replay_cut(&env, env.seed());
    let (_, other) = replay_cut(&env, env.seed().wrapping_add(1));

    assert_ne!(first, other);
}

#[test]
fn a_synthetic_stub_is_served_and_reported_as_synthetic() {
    let env = TestEnv::new();
    let rate_limit = json!({
        "request": { "method": "POST", "path": CHAT, "body": chat_request("busy") },
        "response": {
            "status": 429,
            "headers": { "content-type": "application/json", "retry-after": "7" },
            "body": { "error": { "type": "rate_limit_exceeded" } }
        },
        "synthetic": true
    });
    let recordings = Recordings::serve(&recording(
        &env,
        "faults.json",
        &[chat_exchange("hello", "hi"), rate_limit],
    ))
    .unwrap();

    let ok = post(
        &recordings.base_url(),
        CHAT,
        &chat_request("hello").to_string(),
    );
    let limited = post(
        &recordings.base_url(),
        CHAT,
        &chat_request("busy").to_string(),
    );

    assert_eq!(ok.status, 200);
    assert_eq!(
        (limited.status, limited.retry_after.as_deref()),
        (429, Some("7"))
    );
    assert_eq!(
        recordings.finish(),
        Ok(ServedExchanges {
            recorded: vec![name("faults.json", 0)],
            synthetic: vec![name("faults.json", 1)],
        })
    );
}

#[test]
fn a_recording_file_outside_the_format_is_refused_by_serve_and_by_the_schema() {
    let env = TestEnv::new();
    let good = json!({ "exchanges": [chat_exchange("hello", "hi")] });
    let mut bad = good.clone();
    // An answer under a key the format does not have.
    bad["exchanges"][0]["reply"] = bad["exchanges"][0]["response"].take();
    let folder = recording(&env, "bad.json", &[bad["exchanges"][0].clone()]);
    let schema: Value =
        serde_json::from_str(&std::fs::read_to_string(RECORDING_SCHEMA).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    let served = Recordings::serve(&folder);

    assert!(
        matches!(served, Err(RecordingsError::Format { .. })),
        "{served:?}"
    );
    assert!(!validator.is_valid(&bad));
    assert!(validator.is_valid(&good));
}

#[test]
fn a_folder_without_a_recording_is_refused() {
    let env = TestEnv::new();
    let folder = env.folder().join(env.unique_name("model-exchanges"));
    std::fs::create_dir_all(&folder).unwrap();

    let served = Recordings::serve(&folder);

    assert!(
        matches!(served, Err(RecordingsError::Empty { .. })),
        "{served:?}"
    );
}
