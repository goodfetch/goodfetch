//! The Fake model (ADR 0065), ported from the lab's `fake-llm.ts` and
//! extended: a Model provider on `model.test` that speaks ADR 0012's three
//! formats (OpenAI-compatible chat at `/v1/chat/completions`, Anthropic
//! Messages at `/v1/messages`, Gemini at
//! `/v1beta/models/<model>:generateContent` and `:streamGenerateContent`),
//! streamed and not, with tool calls and structured output.
//!
//! It answers the Proving ground task its request names: the task whose
//! description appears in one of the request's strings, else the task whose
//! host name does. The answer is the task's own at the current time step,
//! shaped as the request asks: a call of the first tool offered (or the one
//! `tool_choice` names) with the tool's parameters filled from it, the
//! requested schema filled from it, or `{"records": […]}` as text. A schema
//! of one of the AI verbs' forms gets the task's answer to that verb
//! (`verbs`); any other is filled from the task's Records. After a tool
//! result it answers in text, but for an Agent step. So its answers depend
//! only on the request, the seed and the time step the task set.
//!
//! It counts each call's tokens itself: a quarter of the characters of
//! every string in the request body, rounded up, in; a quarter of the
//! characters of its answer's content (the text, the refusal, or a tool's
//! name and arguments), rounded up, out. Each call is recorded, with its
//! count, for `GET /model-calls`, so a test checks the engine's Usage
//! against an independent count.
//!
//! A task asks for a fault, never a switch in the binary: a rate limit with
//! `Retry-After`, an overload, a timeout (no answer at all), a cut stream, malformed
//! JSON, a refusal, or a wrong but plausible answer.

mod anthropic;
mod gemini;
mod openai;
mod request;
mod schema;
mod verbs;

use std::fmt::Write as _;

use hyper::StatusCode;
use hyper::body::Bytes;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::sync::Mutex;

use self::request::{Ask, Format, Streaming, ToolChoice};
use self::verbs::Form;
pub(crate) use self::verbs::{Action, Candidate, Element, Locator, Step, Verbs};

/// The host name the Fake model is served on; its base URLs are
/// `http://model.test:<port>/v1` (OpenAI-compatible),
/// `http://model.test:<port>` (Anthropic) and
/// `http://model.test:<port>/v1beta` (Gemini).
pub(crate) const HOST: &str = "model.test";

/// How many characters of content each streamed event carries.
const PIECE: usize = 16;

/// How long a rate-limited client is told to wait, in seconds.
const RETRY_AFTER_SECONDS: u64 = 1;

/// The fault a task asks the Fake model for.
#[derive(Clone)]
pub(crate) enum Fault {
    /// `429` with `Retry-After`.
    RateLimit,
    /// The format's overload: `503`, or Anthropic's `529`.
    Overload,
    /// No answer at all, until the client gives up.
    Timeout,
    /// The answer's body ends before its end: a stream after its first
    /// content event, a whole answer halfway.
    CutStream,
    /// An answer whose JSON (text, output or a tool's arguments) does not
    /// parse.
    MalformedJson,
    /// The format's refusal.
    Refusal,
    /// These answers in place of the task's: wrong, but plausible.
    WrongAnswer(Answers),
}

impl Fault {
    fn name(&self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::Overload => "overload",
            Self::Timeout => "timeout",
            Self::CutStream => "cut_stream",
            Self::MalformedJson => "malformed_json",
            Self::Refusal => "refusal",
            Self::WrongAnswer(_) => "wrong_answer",
        }
    }
}

/// A task the Fake model can be asked about, with its answers at the
/// current time step.
pub(crate) struct Subject {
    pub(crate) id: String,
    pub(crate) description: &'static str,
    pub(crate) host: String,
    pub(crate) fault: Option<Fault>,
    pub(crate) answers: Answers,
}

/// What a task answers at one time step: its Records, and its answer to
/// each AI verb.
#[derive(Clone)]
pub(crate) struct Answers {
    pub(crate) records: Vec<Map<String, Value>>,
    pub(crate) verbs: Verbs,
}

impl Answers {
    /// `schema` filled: with the answer to the AI verb whose form it asks
    /// for, else from the Records.
    fn fill(&self, schema: &Value, ask: &Ask) -> Value {
        Form::asked(schema).map_or_else(
            || schema::fill(schema, &self.records),
            |form| self.verbs.answer(form, &ask.strings, ask.model_turns),
        )
    }
}

/// The Fake model's answer to one call.
pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) content_type: &'static str,
    /// Headers beyond `Content-Type`, such as `Retry-After`.
    pub(crate) headers: Vec<(&'static str, String)>,
    pub(crate) body: Bytes,
    pub(crate) ending: Ending,
}

/// How an answer ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ending {
    /// The body is sent whole.
    Complete,
    /// The body is sent, then the connection is cut before its end.
    Cut,
    /// Nothing is sent until the client gives up.
    Hold,
}

/// One call, as the Fake model counted it.
#[derive(Serialize)]
struct Call {
    sequence: usize,
    format: &'static str,
    model: String,
    stream: bool,
    /// The task the request named, if any.
    task: Option<String>,
    fault: Option<&'static str>,
    input_tokens: u64,
    output_tokens: u64,
}

/// The content of an answer.
pub(crate) enum Content {
    Text(String),
    ToolCall {
        name: String,
        /// The arguments as JSON text.
        arguments: String,
    },
    Refusal(String),
}

const REFUSAL: &str = "I can't help with that request.";

/// What a format's renderer is given.
pub(crate) struct Rendering<'a> {
    pub(crate) ask: &'a Ask,
    /// An ID drawn from the seed and the request.
    pub(crate) id: &'a str,
    pub(crate) content: &'a Content,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
}

/// A rendered stream: its events, each framed, and the index of its first
/// content event.
pub(crate) struct Stream {
    pub(crate) frames: Vec<String>,
    pub(crate) first_content: usize,
}

/// An error a format answers with.
#[derive(Clone, Copy)]
pub(crate) enum Failure {
    /// The request is not one of the format's, or names no task.
    InvalidRequest,
    RateLimit,
    Overload,
}

/// The Fake model of one `serve`.
pub(crate) struct FakeModel {
    seed: u64,
    calls: Mutex<Vec<Call>>,
}

impl FakeModel {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            seed,
            calls: Mutex::default(),
        }
    }

    /// The calls answered so far, as JSON: `{"calls": […]}`.
    pub(crate) fn document(&self) -> Vec<u8> {
        serde_json::to_vec(&json!({ "calls": &*self.calls.lock() }))
            .expect("invariant: the calls serialize")
    }

    /// Answers a `POST` to `target` (path and query) with `body`, about one
    /// of `subjects`; `None` when `target` is none of the formats' paths.
    pub(crate) fn answer(&self, target: &str, body: &[u8], subjects: &[Subject]) -> Option<Answer> {
        let endpoint = request::endpoint(target)?;
        let format = endpoint.format;
        let id = self.id(target, body);
        let Ok(value) = serde_json::from_slice::<Value>(body) else {
            self.record(Call::refused(format, String::new(), false));
            return Some(failure(
                format,
                Failure::InvalidRequest,
                &id,
                "The request body is not JSON.",
            ));
        };
        let ask = request::parse(endpoint, &value);
        let Some(subject) = recognize(&ask, subjects) else {
            self.record(Call::refused(
                format,
                ask.model.clone(),
                ask.streaming.is_some(),
            ));
            return Some(failure(
                format,
                Failure::InvalidRequest,
                &id,
                "The request names no Proving ground task.",
            ));
        };
        let input_tokens = tokens(ask.strings.iter().map(|text| text.chars().count()).sum());
        let fault = subject.fault.as_ref();
        let call = |output_tokens| Call {
            sequence: 0,
            format: format.name(),
            model: ask.model.clone(),
            stream: ask.streaming.is_some(),
            task: Some(subject.id.clone()),
            fault: fault.map(Fault::name),
            input_tokens,
            output_tokens,
        };
        if let Some(answer) = fault.and_then(|fault| refusing(format, fault, &id)) {
            self.record(call(0));
            return Some(answer);
        }
        let answers = match fault {
            Some(Fault::WrongAnswer(answers)) => answers,
            _ => &subject.answers,
        };
        let content = match fault {
            Some(Fault::Refusal) => Content::Refusal(REFUSAL.to_owned()),
            Some(Fault::MalformedJson) => malformed(format, content(&ask, answers)),
            _ => content(&ask, answers),
        };
        let cut = matches!(fault, Some(Fault::CutStream));
        let output_tokens = output_tokens(format, &content);
        // A cut answer delivers its first content event when streamed, and
        // nothing of a whole answer cut halfway.
        self.record(call(match (cut, ask.streaming) {
            (false, _) => output_tokens,
            (true, Some(_)) => tokens(output_text(format, &content).chars().count().min(PIECE)),
            (true, None) => 0,
        }));
        let rendering = Rendering {
            ask: &ask,
            id: &id,
            content: &content,
            input_tokens,
            output_tokens,
        };
        Some(render(format, &rendering, cut))
    }

    fn record(&self, mut call: Call) {
        let mut calls = self.calls.lock();
        call.sequence = calls.len() + 1;
        calls.push(call);
    }

    /// An ID that depends only on the seed and the request.
    fn id(&self, target: &str, body: &[u8]) -> String {
        let digest = Sha256::new()
            .chain_update(self.seed.to_be_bytes())
            .chain_update(target.as_bytes())
            .chain_update(body)
            .finalize();
        digest.iter().take(12).fold(String::new(), |mut id, byte| {
            write!(id, "{byte:02x}").expect("invariant: writing to a String succeeds");
            id
        })
    }
}

impl Call {
    /// A call refused before it named a task.
    fn refused(format: Format, model: String, stream: bool) -> Self {
        Self {
            sequence: 0,
            format: format.name(),
            model,
            stream,
            task: None,
            fault: None,
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

/// The answer of a fault that answers no content: a rate limit, an
/// overload or a timeout.
fn refusing(format: Format, fault: &Fault, id: &str) -> Option<Answer> {
    match fault {
        Fault::RateLimit => {
            let mut answer = failure(
                format,
                Failure::RateLimit,
                id,
                "Rate limit reached; retry after 1 second.",
            );
            answer
                .headers
                .push(("retry-after", RETRY_AFTER_SECONDS.to_string()));
            Some(answer)
        }
        Fault::Overload => Some(failure(
            format,
            Failure::Overload,
            id,
            "The model is overloaded; try again later.",
        )),
        Fault::Timeout => Some(Answer {
            status: StatusCode::OK,
            content_type: "application/json",
            headers: Vec::new(),
            body: Bytes::new(),
            ending: Ending::Hold,
        }),
        _ => None,
    }
}

/// The task a request names: by its description, else by its host name.
fn recognize<'a>(ask: &Ask, subjects: &'a [Subject]) -> Option<&'a Subject> {
    subjects
        .iter()
        .find(|subject| {
            ask.strings
                .iter()
                .any(|text| text.contains(subject.description))
        })
        .or_else(|| {
            subjects.iter().find(|subject| {
                ask.strings
                    .iter()
                    .any(|text| names_host(text, &subject.host))
            })
        })
}

/// Whether `text` names `host` as a whole host name, not as the end of a
/// longer one (`12.agent.test` does not name `2.agent.test`).
fn names_host(text: &str, host: &str) -> bool {
    text.match_indices(host).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + host.len()..].chars().next();
        let bounds =
            |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '.' || c == '-'));
        bounds(before) && bounds(after)
    })
}

/// A quarter of `characters`, rounded up.
fn tokens(characters: usize) -> u64 {
    u64::try_from(characters.div_ceil(4)).unwrap_or(u64::MAX)
}

/// The answer's content, in the form the request asks for.
fn content(ask: &Ask, answers: &Answers) -> Content {
    let offered = match &ask.tool_choice {
        ToolChoice::None => None,
        ToolChoice::Named(name) => ask.tools.iter().find(|tool| &tool.name == name),
        ToolChoice::Auto | ToolChoice::Required => ask.tools.first(),
    };
    // After a tool's result an Agent loop asks for its next step, still a
    // call; any other request gets its answer in text.
    let tool = offered
        .filter(|tool| !ask.after_tool_result || Form::asked(&tool.parameters) == Some(Form::Step));
    if let Some(tool) = tool {
        return Content::ToolCall {
            name: tool.name.clone(),
            arguments: answers.fill(&tool.parameters, ask).to_string(),
        };
    }
    Content::Text(match &ask.schema {
        Some(schema) => answers.fill(schema, ask).to_string(),
        None => json!({ "records": answers.records }).to_string(),
    })
}

/// The first half of the content's JSON, which never parses. Only the
/// OpenAI-compatible format carries a tool's arguments as text; the others
/// answer the malformed JSON as text.
fn malformed(format: Format, content: Content) -> Content {
    let half = |text: &str| {
        let count = text.chars().count();
        text.chars().take((count / 2).max(1)).collect::<String>()
    };
    match content {
        Content::ToolCall { name, arguments } if format == Format::OpenAi => Content::ToolCall {
            name,
            arguments: half(&arguments),
        },
        Content::ToolCall { arguments, .. } => Content::Text(half(&arguments)),
        Content::Text(text) => Content::Text(half(&text)),
        Content::Refusal(text) => Content::Refusal(text),
    }
}

/// The content's text the token count reads: Gemini carries no refusal
/// text.
fn output_text(format: Format, content: &Content) -> String {
    match content {
        Content::Refusal(_) if format == Format::Gemini => String::new(),
        Content::Text(text) | Content::Refusal(text) => text.clone(),
        Content::ToolCall { name, arguments } => format!("{name}{arguments}"),
    }
}

fn output_tokens(format: Format, content: &Content) -> u64 {
    tokens(output_text(format, content).chars().count())
}

/// `text` in streamed pieces of [`PIECE`] characters.
pub(crate) fn pieces(text: &str) -> Vec<String> {
    let characters: Vec<char> = text.chars().collect();
    characters
        .chunks(PIECE)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// The arguments of a tool call as a JSON value, as formats that carry them
/// parsed need them.
pub(crate) fn arguments_value(arguments: &str) -> Value {
    serde_json::from_str(arguments).unwrap_or(Value::Object(Map::new()))
}

fn render(format: Format, rendering: &Rendering<'_>, cut: bool) -> Answer {
    let ask = rendering.ask;
    if ask.streaming == Some(Streaming::JsonArray) {
        // Without `alt=sse`, Gemini streams one JSON array of its chunks.
        let chunks = gemini::chunks(rendering);
        let sent = if cut { &chunks[..1] } else { &chunks[..] };
        let joined: Vec<String> = sent.iter().map(Value::to_string).collect();
        let body = format!("[{}{}", joined.join(",\r\n"), if cut { "" } else { "]" });
        return Answer {
            status: StatusCode::OK,
            content_type: "application/json",
            headers: Vec::new(),
            body: body.into(),
            ending: if cut { Ending::Cut } else { Ending::Complete },
        };
    }
    if ask.streaming.is_some() {
        let stream = match format {
            Format::OpenAi => openai::stream(rendering),
            Format::Anthropic => anthropic::stream(rendering),
            Format::Gemini => gemini::stream(rendering),
        };
        let frames = if cut {
            &stream.frames[..=stream.first_content.min(stream.frames.len() - 1)]
        } else {
            &stream.frames[..]
        };
        return Answer {
            status: StatusCode::OK,
            content_type: "text/event-stream",
            headers: Vec::new(),
            body: frames.concat().into(),
            ending: if cut { Ending::Cut } else { Ending::Complete },
        };
    }
    let value = match format {
        Format::OpenAi => openai::complete(rendering),
        Format::Anthropic => anthropic::complete(rendering),
        Format::Gemini => gemini::complete(rendering),
    };
    let mut body = value.to_string();
    if cut {
        let half = body.len() / 2;
        let at = (0..=half)
            .rev()
            .find(|at| body.is_char_boundary(*at))
            .unwrap_or(0);
        body.truncate(at);
    }
    Answer {
        status: StatusCode::OK,
        content_type: "application/json",
        headers: Vec::new(),
        body: body.into(),
        ending: if cut { Ending::Cut } else { Ending::Complete },
    }
}

fn failure(format: Format, failure: Failure, id: &str, message: &str) -> Answer {
    let (status, value) = match format {
        Format::OpenAi => openai::error(failure, message),
        Format::Anthropic => anthropic::error(failure, id, message),
        Format::Gemini => gemini::error(failure, message),
    };
    Answer {
        status,
        content_type: "application/json",
        headers: Vec::new(),
        body: value.to_string().into(),
        ending: Ending::Complete,
    }
}
