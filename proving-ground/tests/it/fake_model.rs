//! The Fake model (ADR 0065): on `model.test`, it speaks ADR 0012's three
//! formats (OpenAI-compatible chat, Anthropic Messages, Gemini), streamed
//! and not, with tool calls and structured output. It answers the Proving
//! ground task its request names, the same way for the same request and
//! seed, counts each call's tokens itself, and serves the fault a task asks
//! for.
//!
//! Each test reads the answer the way a client of its format does, and takes
//! the expected price from the task's page, never from the Fake model.

use std::time::Duration;

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

const SEED: u64 = 163;

/// The structured output and tool parameters the tests ask for.
fn price_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "price": { "type": "string" } },
        "required": ["price"],
    })
}

#[derive(Clone, Copy, Debug)]
enum Format {
    OpenAi,
    Anthropic,
    Gemini,
}

/// What a request asks for beyond a text answer.
#[derive(Clone, Copy)]
enum Asking {
    Text,
    Tool,
    Structured,
}

fn path(format: Format, stream: bool) -> &'static str {
    match (format, stream) {
        (Format::OpenAi, _) => "/v1/chat/completions",
        (Format::Anthropic, _) => "/v1/messages",
        (Format::Gemini, false) => "/v1beta/models/fake-model:generateContent",
        (Format::Gemini, true) => "/v1beta/models/fake-model:streamGenerateContent?alt=sse",
    }
}

/// A request of `format` whose one user message is `prompt`.
fn request(format: Format, prompt: &str, stream: bool, asking: Asking) -> Value {
    let schema = price_schema();
    let mut body = match format {
        Format::OpenAi => json!({
            "model": "fake-model",
            "messages": [{ "role": "user", "content": prompt }],
            "stream": stream,
        }),
        Format::Anthropic => json!({
            "model": "fake-model",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": prompt }],
            "stream": stream,
        }),
        Format::Gemini => json!({
            "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        }),
    };
    let extra = match (format, asking) {
        (_, Asking::Text) => json!({}),
        (Format::OpenAi, Asking::Tool) => json!({ "tools": [{
            "type": "function",
            "function": { "name": "record_price", "parameters": schema },
        }] }),
        (Format::Anthropic, Asking::Tool) => json!({ "tools": [{
            "name": "record_price",
            "input_schema": schema,
        }] }),
        (Format::Gemini, Asking::Tool) => json!({ "tools": [{ "functionDeclarations": [{
            "name": "record_price",
            "parameters": schema,
        }] }] }),
        (Format::OpenAi, Asking::Structured) => json!({ "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "price", "strict": true, "schema": schema },
        } }),
        (Format::Anthropic, Asking::Structured) => json!({ "output_format": {
            "type": "json_schema",
            "schema": schema,
        } }),
        (Format::Gemini, Asking::Structured) => json!({ "generationConfig": {
            "responseMimeType": "application/json",
            "responseJsonSchema": schema,
        } }),
    };
    if matches!(format, Format::OpenAi) && stream {
        body["stream_options"] = json!({ "include_usage": true });
    }
    for (key, value) in extra.as_object().expect("an object") {
        body[key] = value.clone();
    }
    body
}

/// A model call's response, as sent.
struct Response {
    status: u16,
    retry_after: Option<String>,
    body: Result<String, ureq::Error>,
}

/// Posts `body` to the Fake model at `path`, giving up after `timeout`.
fn call(
    proving_ground: &ProvingGround,
    path: &str,
    body: &Value,
    timeout: Duration,
) -> Result<Response, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build()
        .into();
    let mut response = agent
        .post(format!("http://{}{path}", proving_ground.address()))
        .header("host", "model.test")
        .send_json(body)?;
    Ok(Response {
        status: response.status().as_u16(),
        retry_after: response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        body: response.body_mut().read_to_string(),
    })
}

fn complete_call(proving_ground: &ProvingGround, path: &str, body: &Value) -> (u16, String) {
    let response =
        call(proving_ground, path, body, Duration::from_secs(30)).expect("the Fake model answers");
    (
        response.status,
        response.body.expect("the Fake model sends a whole body"),
    )
}

/// What a client of the format reads from an answer.
#[derive(Debug, Default, PartialEq)]
struct Reading {
    text: String,
    tool: Option<(String, String)>,
    refusal: Option<String>,
    finish: String,
    input_tokens: u64,
    output_tokens: u64,
}

/// The JSON values of a stream's `data:` lines, `[DONE]` aside.
fn events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str(data).expect("each event is JSON"))
        .collect()
}

fn text_of(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn read(format: Format, stream: bool, body: &str) -> Reading {
    let mut reading = Reading::default();
    let values = if stream {
        events(body)
    } else {
        vec![serde_json::from_str(body).expect("the answer is JSON")]
    };
    for value in &values {
        match format {
            Format::OpenAi => read_openai(stream, value, &mut reading),
            Format::Anthropic => read_anthropic(stream, value, &mut reading),
            Format::Gemini => read_gemini(value, &mut reading),
        }
    }
    reading
}

fn read_openai(stream: bool, value: &Value, reading: &mut Reading) {
    if let Some(usage) = value.get("usage").filter(|usage| !usage.is_null()) {
        reading.input_tokens = usage["prompt_tokens"].as_u64().expect("a count");
        reading.output_tokens = usage["completion_tokens"].as_u64().expect("a count");
    }
    let Some(choice) = value["choices"].get(0) else {
        return;
    };
    let message = if stream {
        &choice["delta"]
    } else {
        &choice["message"]
    };
    reading.text.push_str(text_of(&message["content"]));
    if let Some(refusal) = message["refusal"].as_str() {
        reading.refusal.get_or_insert_default().push_str(refusal);
    }
    if let Some(call) = message["tool_calls"].get(0) {
        let tool = reading.tool.get_or_insert_default();
        tool.0.push_str(text_of(&call["function"]["name"]));
        tool.1.push_str(text_of(&call["function"]["arguments"]));
    }
    if let Some(finish) = choice["finish_reason"].as_str() {
        finish.clone_into(&mut reading.finish);
    }
}

fn read_anthropic(stream: bool, value: &Value, reading: &mut Reading) {
    let blocks: Vec<Value> = if stream {
        match text_of(&value["type"]) {
            "message_start" => {
                reading.input_tokens = value["message"]["usage"]["input_tokens"]
                    .as_u64()
                    .expect("a count");
                Vec::new()
            }
            "content_block_start" => vec![value["content_block"].clone()],
            "content_block_delta" => {
                let delta = &value["delta"];
                match text_of(&delta["type"]) {
                    "text_delta" => reading.text.push_str(text_of(&delta["text"])),
                    _ => reading
                        .tool
                        .get_or_insert_default()
                        .1
                        .push_str(text_of(&delta["partial_json"])),
                }
                Vec::new()
            }
            "message_delta" => {
                text_of(&value["delta"]["stop_reason"]).clone_into(&mut reading.finish);
                reading.output_tokens = value["usage"]["output_tokens"].as_u64().expect("a count");
                Vec::new()
            }
            _ => Vec::new(),
        }
    } else {
        reading.input_tokens = value["usage"]["input_tokens"].as_u64().expect("a count");
        reading.output_tokens = value["usage"]["output_tokens"].as_u64().expect("a count");
        text_of(&value["stop_reason"]).clone_into(&mut reading.finish);
        value["content"].as_array().cloned().unwrap_or_default()
    };
    for block in blocks {
        if block["type"] == "text" {
            reading.text.push_str(text_of(&block["text"]));
        } else {
            let tool = reading.tool.get_or_insert_default();
            tool.0.push_str(text_of(&block["name"]));
            if !stream {
                tool.1.push_str(&block["input"].to_string());
            }
        }
    }
}

fn read_gemini(value: &Value, reading: &mut Reading) {
    let usage = &value["usageMetadata"];
    if let Some(count) = usage["candidatesTokenCount"].as_u64() {
        reading.input_tokens = usage["promptTokenCount"].as_u64().expect("a count");
        reading.output_tokens = count;
    }
    let candidate = &value["candidates"][0];
    for part in candidate["content"]["parts"]
        .as_array()
        .into_iter()
        .flatten()
    {
        reading.text.push_str(text_of(&part["text"]));
        if let Some(call) = part.get("functionCall") {
            reading.tool = Some((text_of(&call["name"]).to_owned(), call["args"].to_string()));
        }
    }
    if let Some(finish) = candidate["finishReason"].as_str() {
        finish.clone_into(&mut reading.finish);
    }
}

/// The prompt a tool sends for `task`: its description, as the instruction.
fn prompt(proving_ground: &ProvingGround, task: &str) -> String {
    let description = proving_ground.task(task)["description"]
        .as_str()
        .expect("a description")
        .to_owned();
    format!("Task: {description}")
}

/// The Pro plan's price the start page of `task` shows.
fn page_price(proving_ground: &ProvingGround, task: &str) -> String {
    let url = proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    let page = proving_ground.get(&url).body;
    Regex::new(r#"data-plan="pro">([0-9]+)<"#)
        .expect("a valid pattern")
        .captures(&page)
        .expect("the page shows the Pro price")[1]
        .to_owned()
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false)]
#[case::openai_streamed(Format::OpenAi, true)]
#[case::anthropic(Format::Anthropic, false)]
#[case::anthropic_streamed(Format::Anthropic, true)]
#[case::gemini(Format::Gemini, false)]
#[case::gemini_streamed(Format::Gemini, true)]
fn each_format_answers_the_task_its_prompt_names(#[case] format: Format, #[case] stream: bool) {
    let proving_ground = ProvingGround::start(SEED);
    let price = page_price(&proving_ground, "agent/2");
    let body = request(
        format,
        &prompt(&proving_ground, "agent/2"),
        stream,
        Asking::Text,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, stream), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, stream, &answer);
    let text: Value = serde_json::from_str(&reading.text).expect("the answer is JSON text");
    assert_eq!(
        text,
        json!({ "records": [{ "id": "S-0", "price": price }] }),
        "{answer}"
    );
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false)]
#[case::openai_streamed(Format::OpenAi, true)]
#[case::anthropic(Format::Anthropic, false)]
#[case::anthropic_streamed(Format::Anthropic, true)]
#[case::gemini(Format::Gemini, false)]
#[case::gemini_streamed(Format::Gemini, true)]
fn the_same_request_and_seed_get_the_same_answer(#[case] format: Format, #[case] stream: bool) {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED);
    let body = request(format, &prompt(&first, "agent/2"), stream, Asking::Text);
    let answer = complete_call(&first, path(format, stream), &body);

    let again = complete_call(&second, path(format, stream), &body);

    assert_eq!(answer, again);
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false, "record_price", "tool_calls")]
#[case::openai_streamed(Format::OpenAi, true, "record_price", "tool_calls")]
#[case::anthropic(Format::Anthropic, false, "record_price", "tool_use")]
#[case::anthropic_streamed(Format::Anthropic, true, "record_price", "tool_use")]
#[case::gemini(Format::Gemini, false, "record_price", "STOP")]
#[case::gemini_streamed(Format::Gemini, true, "record_price", "STOP")]
fn a_tool_call_carries_the_tasks_answer_in_its_arguments(
    #[case] format: Format,
    #[case] stream: bool,
    #[case] tool: &str,
    #[case] finish: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let price = page_price(&proving_ground, "agent/2");
    let body = request(
        format,
        &prompt(&proving_ground, "agent/2"),
        stream,
        Asking::Tool,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, stream), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, stream, &answer);
    let (name, arguments) = reading.tool.expect("the answer calls a tool");
    assert_eq!(name, tool, "{answer}");
    let arguments: Value = serde_json::from_str(&arguments).expect("the arguments are JSON");
    assert_eq!(arguments, json!({ "price": price }), "{answer}");
    assert_eq!(reading.finish, finish, "{answer}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, json!([
    { "role": "user", "content": "PROMPT" },
    { "role": "assistant", "content": null, "tool_calls": [{
        "id": "call_1", "type": "function",
        "function": { "name": "record_price", "arguments": "{}" },
    }] },
    { "role": "tool", "tool_call_id": "call_1", "content": "recorded" },
]))]
#[case::anthropic(Format::Anthropic, json!([
    { "role": "user", "content": "PROMPT" },
    { "role": "assistant", "content": [{
        "type": "tool_use", "id": "toolu_1", "name": "record_price", "input": {},
    }] },
    { "role": "user", "content": [{
        "type": "tool_result", "tool_use_id": "toolu_1", "content": "recorded",
    }] },
]))]
#[case::gemini(Format::Gemini, json!([
    { "role": "user", "parts": [{ "text": "PROMPT" }] },
    { "role": "model", "parts": [{ "functionCall": { "name": "record_price", "args": {} } }] },
    { "role": "user", "parts": [{
        "functionResponse": { "name": "record_price", "response": { "result": "recorded" } },
    }] },
]))]
fn after_a_tool_result_the_answer_is_text(#[case] format: Format, #[case] turns: Value) {
    let proving_ground = ProvingGround::start(SEED);
    let turns = turns
        .to_string()
        .replace("PROMPT", &prompt(&proving_ground, "agent/2"));
    let mut body = request(format, "", false, Asking::Tool);
    let key = if matches!(format, Format::Gemini) {
        "contents"
    } else {
        "messages"
    };
    body[key] = serde_json::from_str(&turns).expect("JSON turns");

    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, false, &answer);
    assert_eq!(reading.tool, None, "{answer}");
    assert!(reading.text.contains("\"records\""), "{answer}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false)]
#[case::openai_streamed(Format::OpenAi, true)]
#[case::anthropic(Format::Anthropic, false)]
#[case::anthropic_streamed(Format::Anthropic, true)]
#[case::gemini(Format::Gemini, false)]
#[case::gemini_streamed(Format::Gemini, true)]
fn structured_output_fills_the_requested_schema(#[case] format: Format, #[case] stream: bool) {
    let proving_ground = ProvingGround::start(SEED);
    let price = page_price(&proving_ground, "agent/2");
    let body = request(
        format,
        &prompt(&proving_ground, "agent/2"),
        stream,
        Asking::Structured,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, stream), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, stream, &answer);
    let output: Value = serde_json::from_str(&reading.text).expect("the output is JSON");
    assert_eq!(output, json!({ "price": price }), "{answer}");
}

/// The tokens a call's response reports and the Fake model's own record of
/// it agree with the count ADR 0065's rule gives: a quarter of the
/// characters of every string in the request, rounded up, in, and of the
/// answer's content, out. The prompts are fixed so the counts are literals.
///
/// Covers: story:2-81
#[rstest]
// The tool's strings ("record_price", "object", "string", "price") are 29
// characters in every format.
// "fake-model", "user", the prompt, "function" and the tool's: 10 + 4 + 50
// + 8 + 29 characters.
#[case::openai(Format::OpenAi, false, 26)]
#[case::openai_streamed(Format::OpenAi, true, 26)]
// "fake-model", "user", the prompt and the tool's: 10 + 4 + 50 + 29.
#[case::anthropic(Format::Anthropic, false, 24)]
#[case::anthropic_streamed(Format::Anthropic, true, 24)]
// "user", the prompt and the tool's (the model is in the path): 4 + 50 + 29.
#[case::gemini(Format::Gemini, false, 21)]
#[case::gemini_streamed(Format::Gemini, true, 21)]
fn the_fake_model_counts_each_calls_tokens(
    #[case] format: Format,
    #[case] stream: bool,
    #[case] input_tokens: u64,
) {
    let proving_ground = ProvingGround::start(SEED);
    // Names the task by its host name, 50 characters in all.
    let body = request(
        format,
        "Read the Pro price on http://2.agent.test/demo now",
        stream,
        Asking::Tool,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, stream), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, stream, &answer);
    // The tool's name and its arguments, `record_price` and
    // `{"price":"NN"}`: 12 + 14 characters.
    assert_eq!(
        (reading.input_tokens, reading.output_tokens),
        (input_tokens, 7),
        "{answer}"
    );
    let calls = proving_ground.control("/model-calls").json();
    assert_eq!(calls["calls"][0]["task"], "agent/2", "{calls}");
    assert_eq!(calls["calls"][0]["input_tokens"], input_tokens, "{calls}");
    assert_eq!(calls["calls"][0]["output_tokens"], 7, "{calls}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, 400)]
#[case::anthropic(Format::Anthropic, 400)]
#[case::gemini(Format::Gemini, 400)]
fn a_request_naming_no_task_is_refused(#[case] format: Format, #[case] code: u16) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(format, "What is the weather like?", false, Asking::Text);

    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);

    assert_eq!(status, code, "{answer}");
    let error: Value = serde_json::from_str(&answer).expect("the error is JSON");
    assert!(error["error"].is_object(), "{answer}");
}

/// The fault tasks: each asks the Fake model for one fault.
const RATE_LIMIT: &str = "agent/3";
const OVERLOAD: &str = "agent/4";
const TIMEOUT: &str = "agent/5";
const CUT_STREAM: &str = "agent/6";
const MALFORMED_JSON: &str = "agent/7";
const REFUSAL: &str = "agent/8";
const WRONG_ANSWER: &str = "agent/9";

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, "requests")]
#[case::anthropic(Format::Anthropic, "rate_limit_error")]
#[case::gemini(Format::Gemini, "RESOURCE_EXHAUSTED")]
fn a_rate_limit_fault_answers_429_with_retry_after(#[case] format: Format, #[case] kind: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, RATE_LIMIT),
        false,
        Asking::Text,
    );

    let response = call(
        &proving_ground,
        path(format, false),
        &body,
        Duration::from_secs(30),
    )
    .expect("the Fake model answers");

    assert_eq!(response.status, 429);
    assert_eq!(response.retry_after.as_deref(), Some("1"));
    let body = response.body.expect("a whole body");
    assert!(body.contains(kind), "{body}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, 503, "server_error")]
#[case::anthropic(Format::Anthropic, 529, "overloaded_error")]
#[case::gemini(Format::Gemini, 503, "UNAVAILABLE")]
fn an_overload_fault_answers_the_formats_overload(
    #[case] format: Format,
    #[case] code: u16,
    #[case] kind: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, OVERLOAD),
        false,
        Asking::Text,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);

    assert_eq!(status, code, "{answer}");
    assert!(answer.contains(kind), "{answer}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi)]
#[case::anthropic(Format::Anthropic)]
#[case::gemini(Format::Gemini)]
fn a_timeout_fault_never_answers(#[case] format: Format) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, TIMEOUT),
        false,
        Asking::Text,
    );

    let response = call(
        &proving_ground,
        path(format, false),
        &body,
        Duration::from_millis(500),
    );

    assert!(
        matches!(response, Err(ureq::Error::Timeout(_))),
        "{:?}",
        response.map(|response| response.status)
    );
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, true)]
#[case::openai_whole(Format::OpenAi, false)]
#[case::anthropic(Format::Anthropic, true)]
#[case::anthropic_whole(Format::Anthropic, false)]
#[case::gemini(Format::Gemini, true)]
#[case::gemini_whole(Format::Gemini, false)]
fn a_cut_stream_fault_ends_the_body_before_its_end(#[case] format: Format, #[case] stream: bool) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, CUT_STREAM),
        stream,
        Asking::Text,
    );

    let response = call(
        &proving_ground,
        path(format, stream),
        &body,
        Duration::from_secs(30),
    )
    .expect("the Fake model answers");

    assert_eq!(response.status, 200);
    assert!(response.body.is_err(), "{:?}", response.body);
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, Asking::Text)]
#[case::openai_tool(Format::OpenAi, Asking::Tool)]
#[case::anthropic(Format::Anthropic, Asking::Text)]
#[case::gemini(Format::Gemini, Asking::Structured)]
fn a_malformed_json_fault_answers_json_that_does_not_parse(
    #[case] format: Format,
    #[case] asking: Asking,
) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, MALFORMED_JSON),
        false,
        asking,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, false, &answer);
    let output = reading
        .tool
        .map_or(reading.text, |(_, arguments)| arguments);
    assert!(!output.is_empty(), "{answer}");
    assert!(serde_json::from_str::<Value>(&output).is_err(), "{answer}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, "stop")]
#[case::anthropic(Format::Anthropic, "refusal")]
#[case::gemini(Format::Gemini, "SAFETY")]
fn a_refusal_fault_answers_the_formats_refusal(#[case] format: Format, #[case] finish: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, REFUSAL),
        false,
        Asking::Text,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, false, &answer);
    assert_eq!(reading.finish, finish, "{answer}");
    assert!(!reading.text.contains("records"), "{answer}");
    if matches!(format, Format::OpenAi) {
        assert!(reading.refusal.is_some(), "{answer}");
    }
}

/// A wrong but plausible answer: a price this page shows at no time, which
/// the oracle refutes when a tool reports it as a success.
///
/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi)]
#[case::anthropic(Format::Anthropic)]
#[case::gemini(Format::Gemini)]
fn a_wrong_answer_fault_answers_a_plausible_price_the_oracle_refutes(#[case] format: Format) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        format,
        &prompt(&proving_ground, WRONG_ANSWER),
        false,
        Asking::Text,
    );
    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);
    assert_eq!(status, 200, "{answer}");
    let text: Value =
        serde_json::from_str(&read(format, false, &answer).text).expect("the answer is JSON");
    let records = text["records"].clone();

    let verdict = proving_ground
        .judge(&json!({ "task": WRONG_ANSWER, "status": "success", "records": records }));

    assert_ne!(
        records[0]["price"],
        page_price(&proving_ground, WRONG_ANSWER)
    );
    assert!(
        records[0]["price"]
            .as_str()
            .is_some_and(|price| !price.is_empty())
    );
    assert_eq!(verdict["outcome"], "false_success", "{verdict}");
    assert_eq!(verdict["wrong"][0]["field"], "price", "{verdict}");
}

/// Covers: story:2-81
#[test]
fn a_fault_task_is_answered_normally_by_the_oracle() {
    let proving_ground = ProvingGround::start(SEED);
    let price = page_price(&proving_ground, RATE_LIMIT);

    let verdict = proving_ground.judge(&json!({
        "task": RATE_LIMIT,
        "status": "success",
        "records": [{ "id": "pro", "price": price }],
    }));

    assert_eq!(verdict["outcome"], "verified", "{verdict}");
}
