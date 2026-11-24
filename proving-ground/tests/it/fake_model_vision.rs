//! The Fake model's vision answers (#1540, ADR 0065): a request that
//! carries a screenshot, in any of ADR 0012's three formats, and asks for a
//! `coordinate` is answered with the middle of the task's target in the
//! page's CSS pixels, the same for the same seed; each image counts a fixed
//! number of tokens, whatever its size. So the vision rung and the canvas
//! page are judged without a real model.
//!
//! Each test takes the expected point from the canvas page's own markup,
//! never from the Fake model.
//! Covers: story:2-33

use regex::Regex;
use rstest::rstest;
use serde_json::{Value, json};

use crate::browser;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 1540;

/// The tokens the Fake model counts for each image.
const IMAGE_TOKENS: u64 = 258;

#[derive(Clone, Copy, Debug)]
enum Format {
    OpenAi,
    Anthropic,
    Gemini,
}

/// How the request asks for the coordinate.
#[derive(Clone, Copy, Debug)]
enum Asking {
    Structured,
    Tool,
}

fn coordinate_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "coordinate": {
                "type": ["array", "null"],
                "items": { "type": "integer" },
                "minItems": 2,
                "maxItems": 2,
            },
        },
        "required": ["coordinate"],
    })
}

/// A screenshot's bytes, base64: a PNG signature followed by `size` bytes
/// the Fake model never decodes.
fn screenshot(size: usize) -> String {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend((0..size).map(|index| u8::try_from(index % 251).expect("below 256")));
    data_encoding::BASE64.encode(&bytes)
}

fn path(format: Format) -> &'static str {
    match format {
        Format::OpenAi => "/v1/chat/completions",
        Format::Anthropic => "/v1/messages",
        Format::Gemini => "/v1beta/models/fake-model:generateContent",
    }
}

/// A request of `format` whose one user message is `prompt` and the
/// `images` (base64 PNGs), asking for the coordinate as `asking` says.
fn request(format: Format, prompt: &str, images: &[String], asking: Asking) -> Value {
    let mut body =
        match format {
            Format::OpenAi => {
                let mut parts = vec![json!({ "type": "text", "text": prompt })];
                parts.extend(images.iter().map(|image| {
                    json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:image/png;base64,{image}") },
                    })
                }));
                json!({
                    "model": "fake-model",
                    "messages": [{ "role": "user", "content": parts }],
                })
            }
            Format::Anthropic => {
                let mut parts = vec![json!({ "type": "text", "text": prompt })];
                parts.extend(images.iter().map(|image| {
                    json!({
                        "type": "image",
                        "source": { "type": "base64", "media_type": "image/png", "data": image },
                    })
                }));
                json!({
                    "model": "fake-model",
                    "max_tokens": 1024,
                    "messages": [{ "role": "user", "content": parts }],
                })
            }
            Format::Gemini => {
                let mut parts = vec![json!({ "text": prompt })];
                parts.extend(images.iter().map(
                    |image| json!({ "inlineData": { "mimeType": "image/png", "data": image } }),
                ));
                json!({ "contents": [{ "role": "user", "parts": parts }] })
            }
        };
    let schema = coordinate_schema();
    let extra = match (format, asking) {
        (Format::OpenAi, Asking::Structured) => json!({ "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "point", "strict": true, "schema": schema },
        } }),
        (Format::Anthropic, Asking::Structured) => json!({ "output_format": {
            "type": "json_schema",
            "schema": schema,
        } }),
        (Format::Gemini, Asking::Structured) => json!({ "generationConfig": {
            "responseMimeType": "application/json",
            "responseJsonSchema": schema,
        } }),
        (Format::OpenAi, Asking::Tool) => json!({ "tools": [{
            "type": "function",
            "function": { "name": "click_at", "parameters": schema },
        }] }),
        (Format::Anthropic, Asking::Tool) => json!({ "tools": [{
            "name": "click_at",
            "input_schema": schema,
        }] }),
        (Format::Gemini, Asking::Tool) => json!({ "tools": [{ "functionDeclarations": [{
            "name": "click_at",
            "parametersJsonSchema": schema,
        }] }] }),
    };
    for (key, value) in extra.as_object().expect("an object") {
        body[key] = value.clone();
    }
    body
}

/// The JSON the answer carries, as a client of `format` reads it: the
/// text of a structured answer, or the arguments of its tool call.
fn answered(format: Format, asking: Asking, answer: &Value) -> Value {
    let text = match (format, asking) {
        (Format::OpenAi, Asking::Structured) => answer["choices"][0]["message"]["content"].clone(),
        (Format::OpenAi, Asking::Tool) => {
            answer["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"].clone()
        }
        (Format::Anthropic, Asking::Structured) => answer["content"][0]["text"].clone(),
        (Format::Anthropic, Asking::Tool) => return answer["content"][0]["input"].clone(),
        (Format::Gemini, Asking::Structured) => {
            answer["candidates"][0]["content"]["parts"][0]["text"].clone()
        }
        (Format::Gemini, Asking::Tool) => {
            return answer["candidates"][0]["content"]["parts"][0]["functionCall"]["args"].clone();
        }
    };
    serde_json::from_str(text.as_str().expect("the answer's JSON as text")).expect("JSON")
}

/// Asks the Fake model `body` in `format`, and the JSON its answer carries.
fn ask(proving_ground: &ProvingGround, format: Format, asking: Asking, body: &Value) -> Value {
    let answer = proving_ground.post_json(&proving_ground.url("model.test", path(format)), body);
    assert_eq!(answer.status, 200, "{}", answer.body);
    answered(format, asking, &answer.json())
}

/// The canvas task's prompt: its description, as a vision rung sends it.
fn prompt(proving_ground: &ProvingGround) -> String {
    let description = proving_ground.task("canvas/1")["description"]
        .as_str()
        .expect("a description")
        .to_owned();
    format!("{description} Answer the point to click on the screenshot.")
}

/// The middle of the drawn button in the page's CSS pixels, read from the
/// page's markup: the canvas's place on the page and the button's on the
/// canvas.
fn button_middle(proving_ground: &ProvingGround) -> Value {
    let page = proving_ground
        .get(&browser::start_url(proving_ground, "canvas/1"))
        .body;
    let number = |pattern: &str| -> Vec<u64> {
        let captures = Regex::new(pattern)
            .expect("a pattern")
            .captures(&page)
            .unwrap_or_else(|| panic!("the page matches {pattern}: {page}"));
        captures
            .iter()
            .skip(1)
            .map(|found| found.expect("a group").as_str().parse().expect("a number"))
            .collect()
    };
    let canvas = number(r"#canvas \{ position: absolute; left: (\d+)px; top: (\d+)px; \}");
    let button = number(r#"data-button="(\d+) (\d+) (\d+) (\d+)""#);
    json!([
        canvas[0] + button[0] + button[2] / 2,
        canvas[1] + button[1] + button[3] / 2,
    ])
}

#[rstest]
#[case::openai(Format::OpenAi, Asking::Structured)]
#[case::openai_tool(Format::OpenAi, Asking::Tool)]
#[case::anthropic(Format::Anthropic, Asking::Structured)]
#[case::anthropic_tool(Format::Anthropic, Asking::Tool)]
#[case::gemini(Format::Gemini, Asking::Structured)]
#[case::gemini_tool(Format::Gemini, Asking::Tool)]
fn a_screenshot_request_is_answered_with_the_drawn_buttons_middle(
    #[case] format: Format,
    #[case] asking: Asking,
) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(format, &prompt(&proving_ground), &[screenshot(64)], asking);

    let answer = ask(&proving_ground, format, asking, &body);

    assert_eq!(
        answer,
        json!({ "coordinate": button_middle(&proving_ground) })
    );
}

#[test]
fn the_coordinate_is_the_same_for_a_seed_and_follows_the_seed() {
    let first = ProvingGround::start(SEED);
    let second = ProvingGround::start(SEED + 1);
    let asked = |proving_ground: &ProvingGround| {
        let body = request(
            Format::OpenAi,
            &prompt(proving_ground),
            &[screenshot(64)],
            Asking::Structured,
        );
        ask(proving_ground, Format::OpenAi, Asking::Structured, &body)
    };

    let (once, again, other) = (asked(&first), asked(&first), asked(&second));

    assert_eq!(once, again);
    assert_eq!(other, json!({ "coordinate": button_middle(&second) }));
    assert_ne!(once, other);
}

#[rstest]
#[case::openai(Format::OpenAi)]
#[case::anthropic(Format::Anthropic)]
#[case::gemini(Format::Gemini)]
fn without_a_screenshot_the_coordinate_is_null(#[case] format: Format) {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(format, &prompt(&proving_ground), &[], Asking::Structured);

    let answer = ask(&proving_ground, format, Asking::Structured, &body);

    assert_eq!(answer, json!({ "coordinate": null }));
}

#[test]
fn a_task_with_nothing_drawn_to_point_at_answers_no_coordinate() {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        Format::OpenAi,
        "Read the Pro price on http://2.agent.test/demo now",
        &[screenshot(64)],
        Asking::Structured,
    );

    let answer = ask(&proving_ground, Format::OpenAi, Asking::Structured, &body);

    assert_eq!(answer, json!({ "coordinate": null }));
}

#[rstest]
#[case::openai(Format::OpenAi)]
#[case::anthropic(Format::Anthropic)]
#[case::gemini(Format::Gemini)]
fn each_image_counts_a_fixed_number_of_tokens_whatever_its_size(#[case] format: Format) {
    let proving_ground = ProvingGround::start(SEED);
    let prompt = prompt(&proving_ground);
    for images in [
        Vec::new(),
        vec![screenshot(16)],
        vec![screenshot(200_000)],
        vec![screenshot(16), screenshot(4_000)],
    ] {
        let body = request(format, &prompt, &images, Asking::Structured);
        ask(&proving_ground, format, Asking::Structured, &body);
    }

    let calls = proving_ground.control("/model-calls").json();
    let counted: Vec<(u64, u64)> = calls["calls"]
        .as_array()
        .expect("the calls")
        .iter()
        .map(|call| {
            (
                call["images"].as_u64().expect("an image count"),
                call["input_tokens"].as_u64().expect("a token count"),
            )
        })
        .collect();
    let text = counted[0].1;
    assert_eq!(
        counted,
        [
            (0, text),
            (1, text + IMAGE_TOKENS),
            (1, text + IMAGE_TOKENS),
            (2, text + 2 * IMAGE_TOKENS),
        ],
        "{calls}"
    );
}

#[test]
fn the_formats_usage_reports_the_images_tokens() {
    let proving_ground = ProvingGround::start(SEED);
    let body = request(
        Format::OpenAi,
        &prompt(&proving_ground),
        &[screenshot(64)],
        Asking::Structured,
    );

    let answer = proving_ground
        .post_json(
            &proving_ground.url("model.test", path(Format::OpenAi)),
            &body,
        )
        .json();

    let calls = proving_ground.control("/model-calls").json();
    assert_eq!(
        answer["usage"]["prompt_tokens"], calls["calls"][0]["input_tokens"],
        "{answer}"
    );
}

#[test]
fn a_click_where_the_fake_model_points_on_a_screenshot_is_verified() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = browser::judged(
        &proving_ground,
        "canvas/1",
        "canvas-vision",
        json!({ "prompt": prompt(&proving_ground) }),
    );

    assert_eq!(answer["outcome"], "verified", "{answer}");
}
