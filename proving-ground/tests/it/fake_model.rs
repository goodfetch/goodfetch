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

/// A request of `format` whose one user message is `prompt`, asking for the
/// price.
fn request(format: Format, prompt: &str, stream: bool, asking: Asking) -> Value {
    request_for(
        format,
        prompt,
        stream,
        asking,
        "record_price",
        &price_schema(),
    )
}

/// A request of `format` whose one user message is `prompt`, offering the
/// tool `tool` or asking for structured output, each with `schema`.
fn request_for(
    format: Format,
    prompt: &str,
    stream: bool,
    asking: Asking,
    tool: &str,
    schema: &Value,
) -> Value {
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
            "function": { "name": tool, "parameters": schema },
        }] }),
        (Format::Anthropic, Asking::Tool) => json!({ "tools": [{
            "name": tool,
            "input_schema": schema,
        }] }),
        (Format::Gemini, Asking::Tool) => json!({ "tools": [{ "functionDeclarations": [{
            "name": tool,
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

// The forms the AI verbs ask for (#2338). Each request shows the task's page
// as the engine's prompt does, an `ai` aria snapshot with element
// references. The test gives each element its reference itself, so an
// answer naming the right element names the reference the test gave it.

/// The task whose page holds nothing its instruction asks for.
const NOTHING_FITS: &str = "agent/10";

/// The references the snapshot gives the demo page's elements.
const STARTER_PRICE: &str = "e6";
const PRO_PRICE: &str = "e9";
const FULL_NAME: &str = "e11";
const WORK_EMAIL: &str = "e12";
const PLAN: &str = "e13";
const CONSENT: &str = "e14";
const BOOK: &str = "e15";
const DELETE: &str = "e16";

/// The AI verbs, each asking by the schema its request carries.
#[derive(Clone, Copy, Debug)]
enum Verb {
    Observe,
    Act,
    /// Compiled `extract`, asking for each field's locator.
    Locators,
    /// One step of `page.agent`.
    Step,
}

impl Verb {
    fn tool(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Act => "act",
            Self::Locators => "write_locators",
            Self::Step => "agent_step",
        }
    }

    fn schema(self) -> Value {
        let action = json!({
            "type": "object",
            "properties": {
                "method": { "type": "string" },
                "element": { "type": "string" },
                "arguments": { "type": "array", "items": {} },
            },
        });
        match self {
            Self::Observe => json!({
                "type": "object",
                "properties": { "candidates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "element": { "type": "string" },
                            "description": { "type": "string" },
                        },
                    },
                } },
            }),
            Self::Act => {
                let mut action = action;
                action["type"] = json!(["object", "null"]);
                json!({ "type": "object", "properties": { "action": action } })
            }
            Self::Locators => json!({
                "type": "object",
                "properties": { "locators": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": { "type": "string" },
                            "element": { "type": "string" },
                        },
                    },
                } },
            }),
            Self::Step => json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" },
                    "actions": { "type": "array", "items": action },
                },
            }),
        }
    }
}

/// The start page of `task` at the current time step.
fn start_page(proving_ground: &ProvingGround, task: &str) -> String {
    let url = proving_ground.task(task)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned();
    proving_ground.get(&url).body
}

/// The demo page of `task` at the current time step as an `ai` aria
/// snapshot, with the prices and buttons the page shows.
fn snapshot(proving_ground: &ProvingGround, task: &str) -> String {
    let page = start_page(proving_ground, task);
    let price = |plan: &str| {
        Regex::new(&format!(r#"data-plan="{plan}">([0-9]+)<"#))
            .expect("a valid pattern")
            .captures(&page)
            .expect("the page shows the plan's price")[1]
            .to_owned()
    };
    let (starter, pro) = (price("starter"), price("pro"));
    let mut lines = vec![
        "- table:".to_owned(),
        "  - row \"Plan Price per month\":".to_owned(),
        "    - columnheader \"Plan\" [ref=e3]".to_owned(),
        "    - columnheader \"Price per month\" [ref=e4]".to_owned(),
        format!("  - row \"Starter {starter}\":"),
        "    - cell \"Starter\" [ref=e5]".to_owned(),
        format!("    - cell \"{starter}\" [ref={STARTER_PRICE}]"),
        format!("  - row \"Pro {pro}\":"),
        "    - cell \"Pro\" [ref=e8]".to_owned(),
        format!("    - cell \"{pro}\" [ref={PRO_PRICE}]"),
        format!("- textbox \"Full name\" [ref={FULL_NAME}]"),
        format!("- textbox \"Work email\" [ref={WORK_EMAIL}]"),
        format!("- combobox \"Plan\" [ref={PLAN}]:"),
        "  - option \"Starter\"".to_owned(),
        "  - option \"Pro\"".to_owned(),
        format!("- checkbox \"I agree to be contacted\" [ref={CONSENT}]"),
    ];
    if page.contains("Delete my account") {
        lines.push(format!("- button \"Delete my account\" [ref={DELETE}]"));
    }
    lines.push(format!(
        "- button \"Book the demo\" [ref={BOOK}] [cursor=pointer]"
    ));
    lines.join("\n")
}

/// The prompt an AI verb sends for `task`: its description, then the page.
fn page_prompt(proving_ground: &ProvingGround, task: &str) -> String {
    format!(
        "{}\n\nThe page:\n{}",
        prompt(proving_ground, task),
        snapshot(proving_ground, task)
    )
}

fn verb_request(format: Format, prompt: &str, stream: bool, asking: Asking, verb: Verb) -> Value {
    request_for(format, prompt, stream, asking, verb.tool(), &verb.schema())
}

/// The JSON a verb's answer carries: the tool's arguments, else the text.
fn verb_answer(format: Format, stream: bool, answer: &str) -> Value {
    let reading = read(format, stream, answer);
    let json = reading
        .tool
        .map_or(reading.text, |(_, arguments)| arguments);
    serde_json::from_str(&json).unwrap_or_else(|_| panic!("the answer is JSON: {answer}"))
}

/// Asks the Fake model about `task` in the form `verb` asks for, the page
/// in the prompt, and reads the JSON it answers.
fn ask_verb(
    proving_ground: &ProvingGround,
    task: &str,
    verb: Verb,
    format: Format,
    stream: bool,
    asking: Asking,
) -> Value {
    let body = verb_request(
        format,
        &page_prompt(proving_ground, task),
        stream,
        asking,
        verb,
    );
    let (status, answer) = complete_call(proving_ground, path(format, stream), &body);
    assert_eq!(status, 200, "{answer}");
    verb_answer(format, stream, &answer)
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false, Asking::Structured)]
#[case::openai_streamed(Format::OpenAi, true, Asking::Structured)]
#[case::openai_tool(Format::OpenAi, false, Asking::Tool)]
#[case::anthropic(Format::Anthropic, false, Asking::Structured)]
#[case::anthropic_streamed(Format::Anthropic, true, Asking::Structured)]
#[case::anthropic_tool(Format::Anthropic, false, Asking::Tool)]
#[case::gemini(Format::Gemini, false, Asking::Structured)]
#[case::gemini_streamed(Format::Gemini, true, Asking::Structured)]
#[case::gemini_tool(Format::Gemini, true, Asking::Tool)]
fn observe_answers_a_candidate_naming_the_tasks_target(
    #[case] format: Format,
    #[case] stream: bool,
    #[case] asking: Asking,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = ask_verb(
        &proving_ground,
        "agent/2",
        Verb::Observe,
        format,
        stream,
        asking,
    );

    let candidates = answer["candidates"].as_array().expect("candidates");
    assert_eq!(candidates.len(), 1, "{answer}");
    assert_eq!(candidates[0]["element"], PRO_PRICE, "{answer}");
    assert!(
        candidates[0]["description"]
            .as_str()
            .is_some_and(|description| !description.is_empty()),
        "{answer}"
    );
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false, Asking::Tool)]
#[case::openai_structured(Format::OpenAi, true, Asking::Structured)]
#[case::anthropic(Format::Anthropic, false, Asking::Tool)]
#[case::anthropic_streamed(Format::Anthropic, true, Asking::Tool)]
#[case::gemini(Format::Gemini, false, Asking::Tool)]
#[case::gemini_structured(Format::Gemini, true, Asking::Structured)]
fn act_answers_one_action_on_the_tasks_target(
    #[case] format: Format,
    #[case] stream: bool,
    #[case] asking: Asking,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = ask_verb(
        &proving_ground,
        "agent/1",
        Verb::Act,
        format,
        stream,
        asking,
    );

    assert_eq!(
        answer["action"],
        json!({ "method": "click", "element": BOOK, "arguments": [] }),
        "{answer}"
    );
}

/// The price is on the page; the Record's `id` is not, so it has no
/// locator.
///
/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false, Asking::Structured)]
#[case::openai_tool(Format::OpenAi, true, Asking::Tool)]
#[case::anthropic(Format::Anthropic, false, Asking::Structured)]
#[case::anthropic_tool(Format::Anthropic, true, Asking::Tool)]
#[case::gemini(Format::Gemini, false, Asking::Structured)]
#[case::gemini_tool(Format::Gemini, false, Asking::Tool)]
fn compiled_extract_answers_a_locator_for_each_field_the_page_shows(
    #[case] format: Format,
    #[case] stream: bool,
    #[case] asking: Asking,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = ask_verb(
        &proving_ground,
        "agent/2",
        Verb::Locators,
        format,
        stream,
        asking,
    );

    assert_eq!(
        answer["locators"],
        json!([{ "field": "price", "element": PRO_PRICE }]),
        "{answer}"
    );
}

/// The turns of an Agent loop that took `taken` steps: the prompt, then
/// each step's tool call and its result.
fn agent_turns(format: Format, prompt: &str, taken: usize) -> Value {
    let tool = Verb::Step.tool();
    let mut turns = vec![match format {
        Format::Gemini => json!({ "role": "user", "parts": [{ "text": prompt }] }),
        Format::OpenAi | Format::Anthropic => json!({ "role": "user", "content": prompt }),
    }];
    for step in 0..taken {
        let id = format!("call_{step}");
        turns.extend(match format {
            Format::OpenAi => [
                json!({ "role": "assistant", "content": null, "tool_calls": [{
                    "id": id, "type": "function",
                    "function": { "name": tool, "arguments": "{}" },
                }] }),
                json!({ "role": "tool", "tool_call_id": id, "content": "done" }),
            ],
            Format::Anthropic => [
                json!({ "role": "assistant", "content": [{
                    "type": "tool_use", "id": id, "name": tool, "input": {},
                }] }),
                json!({ "role": "user", "content": [{
                    "type": "tool_result", "tool_use_id": id, "content": "done",
                }] }),
            ],
            Format::Gemini => [
                json!({ "role": "model", "parts": [{ "functionCall": { "name": tool, "args": {} } }] }),
                json!({ "role": "user", "parts": [{
                    "functionResponse": { "name": tool, "response": { "result": "done" } },
                }] }),
            ],
        });
    }
    Value::Array(turns)
}

/// Asks for the next step of an Agent loop on `task` that took `taken`
/// steps, and reads its first action.
fn agent_step(
    proving_ground: &ProvingGround,
    task: &str,
    format: Format,
    stream: bool,
    taken: usize,
) -> Value {
    let mut body = verb_request(format, "", stream, Asking::Tool, Verb::Step);
    let key = if matches!(format, Format::Gemini) {
        "contents"
    } else {
        "messages"
    };
    body[key] = agent_turns(format, &page_prompt(proving_ground, task), taken);
    let (status, answer) = complete_call(proving_ground, path(format, stream), &body);
    assert_eq!(status, 200, "{answer}");
    let step = verb_answer(format, stream, &answer);
    assert!(
        step["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()),
        "{answer}"
    );
    step["actions"][0].clone()
}

/// The booking the description of `agent/1` asks for, one action a step.
fn booking_steps() -> [Value; 5] {
    [
        json!({ "method": "fill", "element": FULL_NAME, "arguments": ["Ada Lovelace"] }),
        json!({ "method": "fill", "element": WORK_EMAIL, "arguments": ["ada@example.test"] }),
        json!({ "method": "select_option", "element": PLAN, "arguments": ["pro"] }),
        json!({ "method": "check", "element": CONSENT, "arguments": [] }),
        json!({ "method": "click", "element": BOOK, "arguments": [] }),
    ]
}

/// Each call answers the step after those the request's turns took.
///
/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false, 0)]
#[case::openai_streamed(Format::OpenAi, true, 2)]
#[case::anthropic(Format::Anthropic, false, 1)]
#[case::anthropic_streamed(Format::Anthropic, true, 3)]
#[case::gemini(Format::Gemini, false, 4)]
#[case::gemini_streamed(Format::Gemini, true, 0)]
fn an_agent_request_answers_the_next_step(
    #[case] format: Format,
    #[case] stream: bool,
    #[case] taken: usize,
) {
    let proving_ground = ProvingGround::start(SEED);

    let action = agent_step(&proving_ground, "agent/1", format, stream, taken);

    assert_eq!(action, booking_steps()[taken]);
}

/// Covers: story:2-81
#[rstest]
#[case::once_booked(5)]
#[case::after_its_end(7)]
fn the_agent_steps_end_with_done(#[case] taken: usize) {
    let proving_ground = ProvingGround::start(SEED);

    let action = agent_step(&proving_ground, "agent/1", Format::OpenAi, false, taken);

    assert_eq!(action["method"], "done", "{action}");
}

/// Covers: story:2-81
#[test]
fn an_agent_step_that_reads_a_price_ends_with_the_tasks_record() {
    let proving_ground = ProvingGround::start(SEED);
    let price = page_price(&proving_ground, "agent/2");

    let action = agent_step(&proving_ground, "agent/2", Format::Anthropic, false, 0);

    assert_eq!(action["method"], "done", "{action}");
    assert_eq!(
        action["arguments"],
        json!([{ "records": [{ "id": "S-0", "price": price }] }]),
        "{action}"
    );
}

/// The elements are the same, but the page changes with the time step: the
/// Pro price changes at step 3, and a delete control takes the booking
/// button's place at step 2.
///
/// Covers: story:2-81
#[rstest]
#[case::price_change("agent/2", 3, Verb::Observe, "/candidates/0/element", PRO_PRICE)]
#[case::price_change_locator("agent/2", 3, Verb::Locators, "/locators/0/element", PRO_PRICE)]
#[case::decoy_swap("agent/1", 2, Verb::Act, "/action/element", BOOK)]
fn a_verb_answer_follows_the_time_step(
    #[case] task: &str,
    #[case] step: u64,
    #[case] verb: Verb,
    #[case] pointer: &str,
    #[case] element: &str,
) {
    let proving_ground = ProvingGround::start(SEED);
    proving_ground.set_time_step(&json!(step));

    let answer = ask_verb(
        &proving_ground,
        task,
        verb,
        Format::OpenAi,
        false,
        Asking::Tool,
    );

    assert_eq!(answer.pointer(pointer), Some(&json!(element)), "{answer}");
}

/// A prompt that shows no element references gets the element's selector on
/// the page at that time step: its renamed `id` at step 1.
///
/// Covers: story:2-81
#[rstest]
#[case::baseline(0)]
#[case::relabel(1)]
fn without_references_an_answer_names_the_elements_selector(#[case] step: u64) {
    let proving_ground = ProvingGround::start(SEED);
    proving_ground.set_time_step(&json!(step));
    let page = start_page(&proving_ground, "agent/1");
    let id = Regex::new(r#"<button id="([^"]+)"[^>]*type="submit">Book the demo"#)
        .expect("a valid pattern")
        .captures(&page)
        .expect("the page has its booking button")[1]
        .to_owned();
    let body = verb_request(
        Format::OpenAi,
        &prompt(&proving_ground, "agent/1"),
        false,
        Asking::Tool,
        Verb::Act,
    );

    let (status, answer) = complete_call(&proving_ground, path(Format::OpenAi, false), &body);

    assert_eq!(status, 200, "{answer}");
    let answer = verb_answer(Format::OpenAi, false, &answer);
    assert_eq!(answer["action"]["element"], format!("#{id}"), "{answer}");
}

/// Covers: story:2-81
#[test]
fn where_nothing_fits_observe_answers_no_candidate() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = ask_verb(
        &proving_ground,
        NOTHING_FITS,
        Verb::Observe,
        Format::OpenAi,
        false,
        Asking::Structured,
    );

    assert_eq!(answer, json!({ "candidates": [] }));
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi)]
#[case::anthropic(Format::Anthropic)]
#[case::gemini(Format::Gemini)]
fn where_nothing_fits_act_answers_no_action(#[case] format: Format) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = ask_verb(
        &proving_ground,
        NOTHING_FITS,
        Verb::Act,
        format,
        false,
        Asking::Structured,
    );

    assert_eq!(answer, json!({ "action": null }));
}

/// Covers: story:2-81
#[test]
fn where_nothing_fits_the_agent_gives_up() {
    let proving_ground = ProvingGround::start(SEED);

    let action = agent_step(&proving_ground, NOTHING_FITS, Format::Gemini, false, 0);

    assert_eq!(action["method"], "give_up", "{action}");
}

/// The task whose page holds nothing that fits is a refusal: nothing
/// carried out, and a reported failure.
///
/// Covers: story:2-81
#[test]
fn the_task_where_nothing_fits_is_verified_by_a_refusal() {
    let proving_ground = ProvingGround::start(SEED);

    let verdict =
        proving_ground.judge(&json!({ "task": NOTHING_FITS, "status": "failure", "records": [] }));

    assert_eq!(verdict["outcome"], "verified", "{verdict}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, Verb::Observe)]
#[case::anthropic(Format::Anthropic, Verb::Locators)]
#[case::gemini(Format::Gemini, Verb::Step)]
fn the_same_verb_request_seed_and_time_step_get_the_same_answer(
    #[case] format: Format,
    #[case] verb: Verb,
) {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED);
    first.set_time_step(&json!(3));
    second.set_time_step(&json!(3));
    let body = verb_request(
        format,
        &page_prompt(&first, "agent/2"),
        true,
        Asking::Tool,
        verb,
    );
    let answer = complete_call(&first, path(format, true), &body);

    let again = complete_call(&second, path(format, true), &body);

    assert_eq!(answer, again);
}

/// The output count is a quarter of the characters of the answer's text,
/// rounded up, as the call's usage and the Fake model's record say.
///
/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, false)]
#[case::openai_streamed(Format::OpenAi, true)]
#[case::anthropic(Format::Anthropic, false)]
#[case::anthropic_streamed(Format::Anthropic, true)]
#[case::gemini(Format::Gemini, false)]
#[case::gemini_streamed(Format::Gemini, true)]
fn the_token_count_covers_a_verb_answer(#[case] format: Format, #[case] stream: bool) {
    let proving_ground = ProvingGround::start(SEED);
    let body = verb_request(
        format,
        &page_prompt(&proving_ground, "agent/2"),
        stream,
        Asking::Structured,
        Verb::Observe,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, stream), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, stream, &answer);
    assert!(reading.text.contains(PRO_PRICE), "{answer}");
    let expected = u64::try_from(reading.text.chars().count().div_ceil(4)).expect("a count");
    assert_eq!(reading.output_tokens, expected, "{answer}");
    let calls = proving_ground.control("/model-calls").json();
    assert_eq!(calls["calls"][0]["output_tokens"], expected, "{calls}");
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi)]
#[case::anthropic(Format::Anthropic)]
#[case::gemini(Format::Gemini)]
fn a_rate_limit_fault_applies_to_a_verb_answer(#[case] format: Format) {
    let proving_ground = ProvingGround::start(SEED);
    let body = verb_request(
        format,
        &page_prompt(&proving_ground, RATE_LIMIT),
        false,
        Asking::Tool,
        Verb::Act,
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
}

/// Covers: story:2-81
#[rstest]
#[case::openai(Format::OpenAi, Asking::Tool)]
#[case::anthropic(Format::Anthropic, Asking::Structured)]
#[case::gemini(Format::Gemini, Asking::Structured)]
fn a_malformed_json_fault_applies_to_a_verb_answer(#[case] format: Format, #[case] asking: Asking) {
    let proving_ground = ProvingGround::start(SEED);
    let body = verb_request(
        format,
        &page_prompt(&proving_ground, MALFORMED_JSON),
        false,
        asking,
        Verb::Observe,
    );

    let (status, answer) = complete_call(&proving_ground, path(format, false), &body);

    assert_eq!(status, 200, "{answer}");
    let reading = read(format, false, &answer);
    let output = reading
        .tool
        .map_or(reading.text, |(_, arguments)| arguments);
    assert!(output.contains("candidates"), "{answer}");
    assert!(serde_json::from_str::<Value>(&output).is_err(), "{answer}");
}

/// A wrong but plausible answer names the Starter plan's price where the
/// task asks for the Pro plan's.
///
/// Covers: story:2-81
#[rstest]
#[case::observe(Verb::Observe, "/candidates/0/element")]
#[case::locators(Verb::Locators, "/locators/0/element")]
fn a_wrong_answer_fault_names_a_plausible_wrong_element(#[case] verb: Verb, #[case] pointer: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = ask_verb(
        &proving_ground,
        WRONG_ANSWER,
        verb,
        Format::Anthropic,
        false,
        Asking::Tool,
    );

    assert_eq!(
        answer.pointer(pointer),
        Some(&json!(STARTER_PRICE)),
        "{answer}"
    );
}
