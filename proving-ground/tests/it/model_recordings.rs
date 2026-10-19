//! The Fake model keeps the shape of the committed Model recordings of each
//! format (ADR 0065): for every recorded exchange, the Fake model's answer
//! to the recorded request has the recording's fields with the same types,
//! the same stream event kinds in the same order, and the same error body.
//! Content is never compared.
//!
//! A Model recording is a JSON file of exchanges, `{"request": {"method",
//! "path", "body"}, "response": {"status", "headers", "body" | "chunks"}}`
//! (#407), one exchange, an array of them or `{"exchanges": […]}`, in a
//! `recordings/` folder beside its adapter's tests. The check reads every
//! such file in the repository, so a recording joins it when committed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

/// Folders the search never enters: version control, build outputs,
/// installed packages and the frozen lab.
const SKIPPED: [&str; 5] = [".git", "target", "node_modules", ".venv", "rnd"];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the Proving ground sits in the repository")
        .to_owned()
}

/// Every JSON file in a `recordings/` folder under `folder`.
fn recording_files(folder: &Path, inside: bool, files: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .expect("the folder is readable")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        if path.is_dir() {
            if !SKIPPED.contains(&name.as_str()) {
                recording_files(&path, inside || name == "recordings", files);
            }
        } else if inside
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
}

/// The exchanges of one recording file.
fn exchanges(file: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(file).expect("the recording is readable");
    let value: Value = serde_json::from_str(&text).expect("the recording is JSON");
    match value {
        Value::Array(exchanges) => exchanges,
        Value::Object(ref fields) if fields.contains_key("exchanges") => {
            value["exchanges"].as_array().cloned().unwrap_or_default()
        }
        exchange => vec![exchange],
    }
}

/// A JSON value's shape: its type, an object's fields with theirs, and the
/// shapes an array's items take.
fn shape(value: &Value) -> Value {
    match value {
        Value::Null => json!("null"),
        Value::Bool(_) => json!("boolean"),
        Value::Number(_) => json!("number"),
        Value::String(_) => json!("string"),
        Value::Array(items) => {
            let mut merged = BTreeMap::new();
            for item in items {
                merged.insert(shape(item).to_string(), shape(item));
            }
            json!({ "array": merged.into_values().collect::<Vec<_>>() })
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), shape(value)))
                .collect(),
        ),
    }
}

/// Where `fake`'s shape differs from `recorded`'s, each as a path and the
/// two shapes. A `null` on either side matches any type, as providers send
/// `null` for a field they leave unset.
fn differences(path: &str, recorded: &Value, fake: &Value, found: &mut Vec<String>) {
    match (recorded, fake) {
        (Value::String(kind), _) | (_, Value::String(kind)) if kind == "null" => {}
        (Value::Object(recorded), Value::Object(fake)) => {
            for (name, value) in recorded {
                match fake.get(name) {
                    Some(other) => differences(&format!("{path}.{name}"), value, other, found),
                    None => found.push(format!("{path}.{name}: missing from the Fake model")),
                }
            }
            for name in fake.keys().filter(|name| !recorded.contains_key(*name)) {
                found.push(format!("{path}.{name}: not in the recording"));
            }
        }
        (Value::Array(recorded), Value::Array(fake)) if recorded.len() == 1 && fake.len() == 1 => {
            differences(&format!("{path}[]"), &recorded[0], &fake[0], found);
        }
        _ if recorded == fake => {}
        _ => found.push(format!(
            "{path}: recorded {recorded}, the Fake model {fake}"
        )),
    }
}

/// A response's body as text: its `body`, or its `chunks` joined.
fn body_text(response: &Value) -> String {
    match &response["chunks"] {
        Value::Array(chunks) => chunks.iter().filter_map(Value::as_str).collect(),
        _ => match &response["body"] {
            Value::String(text) => text.clone(),
            body => body.to_string(),
        },
    }
}

/// A body's events: each with its kind (an event's name or `type`, or what
/// an OpenAI-compatible or Gemini chunk carries) and its JSON; a whole JSON
/// body is one event of kind `body`.
fn events(text: &str) -> Vec<(String, Value)> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return vec![("body".to_owned(), value)];
    }
    let mut events = Vec::new();
    let mut name = None;
    for line in text.lines().map(|line| line.trim_end_matches('\r')) {
        if let Some(event) = line.strip_prefix("event: ") {
            name = Some(event.to_owned());
        } else if let Some(data) = line.strip_prefix("data: ") {
            let value = serde_json::from_str(data).unwrap_or_else(|_| json!(data));
            let kind = name.take().unwrap_or_else(|| chunk_kind(&value));
            events.push((kind, value));
        }
    }
    events
}

fn chunk_kind(value: &Value) -> String {
    if let Some(kind) = value["type"].as_str() {
        return kind.to_owned();
    }
    if value.is_string() {
        return value.as_str().unwrap_or_default().to_owned();
    }
    let delta = &value["choices"][0]["delta"];
    let candidate = &value["candidates"][0];
    let kind = if value["choices"] == json!([]) {
        "usage"
    } else if !value["choices"][0]["finish_reason"].is_null() {
        "finish"
    } else if delta.get("role").is_some() {
        "role"
    } else if delta.get("tool_calls").is_some() {
        "tool_call"
    } else if delta.is_object() {
        "content"
    } else if candidate.get("finishReason").is_some() {
        "finish"
    } else {
        "content"
    };
    kind.to_owned()
}

/// The event kinds in order, a run of one kind counted once.
fn kinds(events: &[(String, Value)]) -> Vec<String> {
    let mut kinds: Vec<String> = events.iter().map(|(kind, _)| kind.clone()).collect();
    kinds.dedup();
    kinds
}

/// The Fake model's answer to a request: its status and body.
fn ask(proving_ground: &ProvingGround, path: &str, body: &Value) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent
        .post(format!("http://{}{path}", proving_ground.address()))
        .header("host", "model.test")
        .send_json(body)
        .expect("the Fake model answers");
    let status = response.status().as_u16();
    let text = response.body_mut().read_to_string().expect("a whole body");
    (status, text)
}

/// Where the Fake model's answer to a recorded exchange's request differs in
/// shape from the recorded response.
fn check(proving_ground: &ProvingGround, label: &str, exchange: &Value) -> Vec<String> {
    let request = &exchange["request"];
    let path = request["path"].as_str().expect("a recorded path");
    let body = match &request["body"] {
        Value::String(text) => serde_json::from_str(text).expect("a JSON request body"),
        body => body.clone(),
    };
    let recorded_status = exchange["response"]["status"].as_u64().expect("a status");
    let recorded = events(&body_text(&exchange["response"]));
    let (status, text) = if recorded_status < 400 {
        ask(proving_ground, path, &body)
    } else {
        // An error body has one shape per format: compare it with the Fake
        // model's own error, to a request naming no task.
        ask(
            proving_ground,
            path,
            &json!({ "model": "fake-model", "messages": [] }),
        )
    };
    let fake = events(&text);
    let mut found = Vec::new();
    if (recorded_status < 400) != (u64::from(status) < 400) {
        found.push(format!(
            "{label}: recorded status {recorded_status}, the Fake model {status}"
        ));
        return found;
    }
    if kinds(&recorded) != kinds(&fake) {
        found.push(format!(
            "{label}: recorded events {:?}, the Fake model {:?}",
            kinds(&recorded),
            kinds(&fake)
        ));
        return found;
    }
    for (kind, value) in &recorded {
        if let Some((_, other)) = fake.iter().find(|(other, _)| other == kind) {
            differences(
                &format!("{label} {kind}"),
                &shape(value),
                &shape(other),
                &mut found,
            );
        }
    }
    found
}

/// Covers: story:2-81
#[test]
fn the_fake_model_keeps_the_shape_of_every_committed_model_recording() {
    let proving_ground = ProvingGround::start(163);
    let mut files = Vec::new();
    recording_files(&repository(), false, &mut files);

    let found: Vec<String> = files
        .iter()
        .flat_map(|file| {
            exchanges(file)
                .iter()
                .enumerate()
                .flat_map(|(at, exchange)| {
                    check(
                        &proving_ground,
                        &format!("{}#{at}", file.display()),
                        exchange,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();

    assert!(found.is_empty(), "{}", found.join("\n"));
}

/// The check reports a recorded field whose type the Fake model's answer
/// does not keep.
///
/// Covers: story:2-81
#[test]
fn a_recording_of_another_shape_is_reported() {
    let proving_ground = ProvingGround::start(163);
    let request = json!({
        "model": "fake-model",
        "messages": [{ "role": "user", "content": "Read the price on http://2.agent.test/demo" }],
    });
    let (_, answer) = ask(&proving_ground, "/v1/chat/completions", &request);
    let mut response: Value = serde_json::from_str(&answer).expect("JSON");
    response["usage"]["prompt_tokens"] = json!("26");
    response["choices"][0]["message"]["audio"] = json!({ "id": "audio_1" });
    let exchange = json!({
        "request": { "method": "POST", "path": "/v1/chat/completions", "body": request },
        "response": { "status": 200, "headers": {}, "body": response },
    });

    let found = check(&proving_ground, "recording", &exchange);

    assert_eq!(
        found,
        [
            "recording body.choices.array[].message.audio: missing from the Fake model",
            r#"recording body.usage.prompt_tokens: recorded "string", the Fake model "number""#,
        ]
    );
}

/// The check passes a recording whose fields, types and events the Fake
/// model's answer keeps, whatever its content.
///
/// Covers: story:2-81
#[test]
fn a_recording_of_the_same_shape_passes() {
    let proving_ground = ProvingGround::start(163);
    let request = json!({
        "model": "fake-model",
        "max_tokens": 64,
        "stream": true,
        "messages": [{ "role": "user", "content": "Read the price on http://2.agent.test/demo" }],
    });
    // A stream as Anthropic sends one, with other content and counts.
    let chunks = [
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude\",\"content\":[],\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{\"input_tokens\":9,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":1,\"service_tier\":\"standard\"}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" there\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":3}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    ];
    let exchange = json!({
        "request": { "method": "POST", "path": "/v1/messages", "body": request },
        "response": { "status": 200, "headers": {}, "chunks": chunks },
    });

    let found = check(&proving_ground, "recording", &exchange);

    assert_eq!(found, Vec::<String>::new());
}
