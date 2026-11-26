//! What a model request asks, read from each format's request body: the
//! model, whether and how to stream, the tools offered and the choice among
//! them, whether the last turn is a tool's result, the structured output's
//! schema, every string the body holds and how many images it carries.

use serde_json::Value;

/// One of ADR 0012's three formats.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Format {
    /// OpenAI-compatible chat completions.
    OpenAi,
    /// Anthropic Messages.
    Anthropic,
    /// Gemini's `generateContent`.
    Gemini,
}

impl Format {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }
}

/// How a streamed answer is sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Streaming {
    /// Server-sent events.
    Events,
    /// Server-sent events ending with a chunk of the usage, as the
    /// OpenAI-compatible format's `stream_options.include_usage` asks.
    EventsWithUsage,
    /// One JSON array of chunks: Gemini's stream without `alt=sse`.
    JsonArray,
}

/// A tool a request offers.
pub(crate) struct Tool {
    pub(crate) name: String,
    /// Its parameters' JSON Schema.
    pub(crate) parameters: Value,
}

/// Which tool, if any, the request lets the model call.
pub(crate) enum ToolChoice {
    Auto,
    None,
    Required,
    Named(String),
}

/// What one request asks.
pub(crate) struct Ask {
    pub(crate) model: String,
    /// How to stream the answer; `None` for a whole answer.
    pub(crate) streaming: Option<Streaming>,
    pub(crate) tools: Vec<Tool>,
    pub(crate) tool_choice: ToolChoice,
    pub(crate) after_tool_result: bool,
    /// How many of the turns are the model's own: the steps an Agent loop
    /// has taken.
    pub(crate) model_turns: usize,
    /// The structured output's schema, if the request asks for one.
    pub(crate) schema: Option<Value>,
    /// Every string the body holds, in any order, but an image's.
    pub(crate) strings: Vec<String>,
    /// How many images the body carries: the OpenAI-compatible format's
    /// `image_url` parts, Anthropic's `image` blocks and Gemini's parts of
    /// image `inlineData`.
    pub(crate) images: usize,
}

/// What a body holds beyond its fields' meaning: its strings, an image's
/// aside, and its images.
#[derive(Default)]
struct Held {
    strings: Vec<String>,
    images: usize,
}

/// Where a request was sent: its format, and for Gemini, whose path names
/// them, the model and the streaming.
pub(crate) struct Endpoint {
    pub(crate) format: Format,
    model: Option<String>,
    streaming: Option<Streaming>,
}

/// The endpoint a `POST` target names; `None` for another target.
pub(crate) fn endpoint(target: &str) -> Option<Endpoint> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let plain = |format| Endpoint {
        format,
        model: None,
        streaming: None,
    };
    match path {
        "/v1/chat/completions" => Some(plain(Format::OpenAi)),
        "/v1/messages" => Some(plain(Format::Anthropic)),
        _ => {
            let (model, method) = path.strip_prefix("/v1beta/models/")?.split_once(':')?;
            let streaming = match method {
                "generateContent" => None,
                "streamGenerateContent" if query.split('&').any(|pair| pair == "alt=sse") => {
                    Some(Streaming::Events)
                }
                "streamGenerateContent" => Some(Streaming::JsonArray),
                _ => return None,
            };
            Some(Endpoint {
                format: Format::Gemini,
                model: Some(model.to_owned()),
                streaming,
            })
        }
    }
}

/// Reads what `body`, sent to `endpoint`, asks.
pub(crate) fn parse(endpoint: Endpoint, body: &Value) -> Ask {
    let mut held = Held::default();
    collect(endpoint.format, body, &mut held);
    match endpoint.format {
        Format::OpenAi => openai(body, held),
        Format::Anthropic => anthropic(body, held),
        Format::Gemini => gemini(endpoint, body, held),
    }
}

fn text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn openai(body: &Value, held: Held) -> Ask {
    let streaming = body["stream"].as_bool().unwrap_or(false).then(|| {
        if body["stream_options"]["include_usage"] == true {
            Streaming::EventsWithUsage
        } else {
            Streaming::Events
        }
    });
    Ask {
        model: text(&body["model"]).unwrap_or_default(),
        streaming,
        tools: each(&body["tools"])
            .map(|tool| Tool {
                name: text(&tool["function"]["name"]).unwrap_or_default(),
                parameters: tool["function"]["parameters"].clone(),
            })
            .collect(),
        tool_choice: match &body["tool_choice"] {
            Value::String(choice) if choice == "none" => ToolChoice::None,
            Value::String(choice) if choice == "required" => ToolChoice::Required,
            Value::Object(choice) => choice
                .get("function")
                .and_then(|function| text(&function["name"]))
                .map_or(ToolChoice::Auto, ToolChoice::Named),
            _ => ToolChoice::Auto,
        },
        after_tool_result: each(&body["messages"])
            .last()
            .is_some_and(|message| message["role"] == "tool"),
        model_turns: turns(&body["messages"], "assistant"),
        schema: (body["response_format"]["type"] == "json_schema")
            .then(|| body["response_format"]["json_schema"]["schema"].clone()),
        strings: held.strings,
        images: held.images,
    }
}

fn anthropic(body: &Value, held: Held) -> Ask {
    Ask {
        model: text(&body["model"]).unwrap_or_default(),
        streaming: (body["stream"] == true).then_some(Streaming::Events),
        tools: each(&body["tools"])
            .map(|tool| Tool {
                name: text(&tool["name"]).unwrap_or_default(),
                parameters: tool["input_schema"].clone(),
            })
            .collect(),
        tool_choice: match body["tool_choice"]["type"].as_str() {
            Some("none") => ToolChoice::None,
            Some("any") => ToolChoice::Required,
            Some("tool") => {
                text(&body["tool_choice"]["name"]).map_or(ToolChoice::Required, ToolChoice::Named)
            }
            _ => ToolChoice::Auto,
        },
        after_tool_result: each(&body["messages"]).last().is_some_and(|message| {
            each(&message["content"]).any(|block| block["type"] == "tool_result")
        }),
        model_turns: turns(&body["messages"], "assistant"),
        schema: [&body["output_format"], &body["output_config"]["format"]]
            .into_iter()
            .find(|format| format["type"] == "json_schema")
            .map(|format| format["schema"].clone()),
        strings: held.strings,
        images: held.images,
    }
}

fn gemini(endpoint: Endpoint, body: &Value, held: Held) -> Ask {
    let calling = &body["toolConfig"]["functionCallingConfig"];
    let allowed: Vec<String> = each(&calling["allowedFunctionNames"])
        .filter_map(text)
        .collect();
    let config = &body["generationConfig"];
    Ask {
        model: endpoint.model.unwrap_or_default(),
        streaming: endpoint.streaming,
        tools: each(&body["tools"])
            .flat_map(|tool| each(&tool["functionDeclarations"]))
            .map(|declaration| Tool {
                name: text(&declaration["name"]).unwrap_or_default(),
                parameters: if declaration["parametersJsonSchema"].is_null() {
                    declaration["parameters"].clone()
                } else {
                    declaration["parametersJsonSchema"].clone()
                },
            })
            .collect(),
        tool_choice: match calling["mode"].as_str() {
            Some("NONE") => ToolChoice::None,
            Some("ANY") => match allowed.as_slice() {
                [name] => ToolChoice::Named(name.clone()),
                _ => ToolChoice::Required,
            },
            _ => ToolChoice::Auto,
        },
        after_tool_result: each(&body["contents"]).last().is_some_and(|content| {
            each(&content["parts"]).any(|part| part.get("functionResponse").is_some())
        }),
        model_turns: turns(&body["contents"], "model"),
        schema: ["responseJsonSchema", "responseSchema"]
            .into_iter()
            .map(|key| &config[key])
            .find(|schema| !schema.is_null())
            .cloned(),
        strings: held.strings,
        images: held.images,
    }
}

/// How many of the turns in `turns` have the role `role`.
fn turns(turns: &Value, role: &str) -> usize {
    each(turns).filter(|turn| turn["role"] == role).count()
}

/// The elements of an array, none for anything else.
fn each(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

/// Collects into `held` the strings of `value`, a body of `format`, and its
/// images, whose strings (their bytes or their URL) it leaves out.
fn collect(format: Format, value: &Value, held: &mut Held) {
    match value {
        Value::String(text) => held.strings.push(text.clone()),
        Value::Array(values) => values.iter().for_each(|value| collect(format, value, held)),
        Value::Object(_) if is_image(format, value) => held.images += 1,
        Value::Object(fields) => fields
            .values()
            .for_each(|value| collect(format, value, held)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Whether `value` is an image of a body of `format`: an `image_url` part,
/// an `image` block, or a part of `inlineData` whose type is an image's.
fn is_image(format: Format, value: &Value) -> bool {
    match format {
        Format::OpenAi => value["type"] == "image_url",
        Format::Anthropic => value["type"] == "image",
        Format::Gemini => ["inlineData", "inline_data"]
            .into_iter()
            .map(|key| &value[key])
            .find(|data| !data.is_null())
            .is_some_and(|data| {
                [&data["mimeType"], &data["mime_type"]]
                    .into_iter()
                    .find_map(Value::as_str)
                    .is_some_and(|kind| kind.starts_with("image/"))
            }),
    }
}
