//! Newline-delimited JSON-RPC framing, without I/O: bytes in, lines out, and
//! each line decoded into a request, a notification or the reply to a handler
//! request the engine sent. A line longer than
//! [`MAX_MESSAGE_BYTES`] or nested deeper than [`MAX_DEPTH`] ends in its
//! typed limit error (ADR 0067), and the framer keeps no more than the limit
//! in memory while it skips the rest of that line.
//!
//! A line is decoded in one pass, which reads its envelope and only finds
//! where its params, result or error are (`serde_json`'s `RawValue`): the
//! message keeps its line, and its handler reads the params straight into
//! their type from where they are, as a reply's reader reads its result,
//! never through a `Value` or a copy (ADR 0140's E1).

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject, ProtocolFailure, RequestId};
use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Map;
use serde_json::value::RawValue;

use crate::wire::Key;
pub use crate::wire::Received;

/// The longest message the engine reads, in bytes, its newline excluded.
/// Page bodies and files travel as handles, never inline.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// The deepest nesting of arrays and objects the engine reads.
pub const MAX_DEPTH: usize = 64;

/// One line the framer cut.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// A whole line, its newline removed.
    Line(Vec<u8>),
    /// A line longer than the framer's limit, skipped.
    TooLarge,
}

/// Cuts a byte stream into lines.
#[derive(Debug)]
pub struct Framer {
    line: Vec<u8>,
    oversized: bool,
    limit: usize,
}

impl Framer {
    /// A framer that refuses lines longer than `limit` bytes.
    pub fn new(limit: usize) -> Self {
        Self {
            line: Vec::new(),
            oversized: false,
            limit,
        }
    }

    /// Takes the next bytes of the stream and appends each line they end to
    /// `frames`. A line holding only whitespace is no frame.
    pub fn feed(&mut self, mut bytes: &[u8], frames: &mut Vec<Frame>) {
        while let Some(end) = bytes.iter().position(|byte| *byte == b'\n') {
            self.take(&bytes[..end]);
            frames.extend(self.end_line());
            bytes = &bytes[end + 1..];
        }
        self.take(bytes);
    }

    /// The stream ended: the last line, if it had no newline.
    pub fn finish(&mut self) -> Option<Frame> {
        self.end_line()
    }

    fn take(&mut self, part: &[u8]) {
        if self.oversized {
            return;
        }
        if self.line.len() + part.len() > self.limit {
            self.oversized = true;
            self.line = Vec::new();
        } else {
            self.line.extend_from_slice(part);
        }
    }

    fn end_line(&mut self) -> Option<Frame> {
        if std::mem::take(&mut self.oversized) {
            return Some(Frame::TooLarge);
        }
        let line = std::mem::take(&mut self.line);
        (!line.iter().all(u8::is_ascii_whitespace)).then_some(Frame::Line(line))
    }
}

/// A message a Client sends.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// A call.
    Request {
        /// Its id.
        id: RequestId,
        /// Its method and params.
        call: Call,
    },
    /// A message that expects no reply.
    Notification(Call),
    /// The reply to a handler request the engine sent.
    Reply {
        /// The request's id.
        id: RequestId,
        /// Its result, where it is in its line, or its failure.
        answer: Result<Received, ErrorObject>,
    },
}

/// A request's or a notification's method and params, in the line they
/// came on.
#[derive(Debug, PartialEq)]
pub struct Call {
    line: String,
    method: Text,
    /// Where the params are; none when the message has none, or `null`.
    params: Option<Range<usize>>,
}

impl Call {
    /// Its method.
    pub fn method(&self) -> &str {
        match &self.method {
            Text::At(range) => &self.line[range.clone()],
            Text::Unescaped(method) => method,
        }
    }

    /// Its params' JSON text, where they are in the line: none when the
    /// message has none, or `null`.
    pub fn params(&self) -> Option<&str> {
        self.params.clone().map(|range| &self.line[range])
    }

    /// Its params, keeping the line they are in: `null` when it has none.
    pub fn into_params(self) -> Received {
        match self.params {
            Some(range) => Received::new(self.line, range),
            None => Received::new("null".to_owned(), 0..4),
        }
    }
}

/// A string member of the envelope: where it is in the line, or, when it
/// holds an escape, its text.
#[derive(Debug, PartialEq)]
enum Text {
    At(Range<usize>),
    Unescaped(String),
}

/// A line that is no message the engine reads.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{failure:?}: {detail}")]
pub struct DecodeError {
    /// The protocol failure it answers with.
    pub failure: ProtocolFailure,
    /// The id of the request, when the line holds a readable one.
    pub id: Option<RequestId>,
    /// What is wrong, for people.
    pub detail: String,
}

impl DecodeError {
    fn new(failure: ProtocolFailure, id: Option<RequestId>, detail: impl Into<String>) -> Self {
        Self {
            failure,
            id,
            detail: detail.into(),
        }
    }

    /// The error of a line the framer skipped as too large.
    pub fn too_large() -> Self {
        Self::new(
            ProtocolFailure::MessageTooLarge,
            None,
            format!("a message is longer than {MAX_MESSAGE_BYTES} bytes"),
        )
    }
}

/// The members a request may carry.
const REQUEST_FIELDS: &[&str] = &["jsonrpc", "id", "method", "params"];

/// The members a notification may carry.
const NOTIFICATION_FIELDS: &[&str] = &["jsonrpc", "method", "params"];

/// Decodes one line into a request, a notification or a reply, which keeps
/// the line: one pass reads its envelope, and its params, result or error
/// are read later from where they are.
pub fn decode(line: Vec<u8>) -> Result<Incoming, DecodeError> {
    if nesting_exceeds(&line, MAX_DEPTH) {
        return Err(DecodeError::new(
            ProtocolFailure::MessageTooDeep,
            None,
            format!("a message nests arrays and objects deeper than {MAX_DEPTH}"),
        ));
    }
    let parse_error = |detail: String| DecodeError::new(ProtocolFailure::ParseError, None, detail);
    let line = String::from_utf8(line).map_err(|error| parse_error(error.to_string()))?;
    let shape =
        serde_json::from_str::<Shape<'_>>(&line).map_err(|error| parse_error(error.to_string()))?;
    let members = match shape {
        Shape::Object(members) => members,
        Shape::Batch => {
            return Err(DecodeError::new(
                ProtocolFailure::InvalidRequest,
                None,
                "a batch, which the protocol does not take",
            ));
        }
        Shape::Scalar => {
            return Err(DecodeError::new(
                ProtocolFailure::InvalidRequest,
                None,
                "a message is a JSON object",
            ));
        }
    };
    let envelope = Envelope::of(&members);
    let id = envelope.id.and_then(request_id);
    if envelope.method.is_none()
        && let Some(id) = id.clone()
        && let Some(answer) = envelope.answer(&line)
    {
        drop(members);
        let answer = answer.map(|range| Received::new(line, range));
        return Ok(Incoming::Reply { id, answer });
    }
    let Some(method) = envelope.method else {
        return Err(DecodeError::new(
            ProtocolFailure::InvalidRequest,
            id,
            "a message to the engine has a `method`",
        ));
    };
    let has_id = envelope.id.is_some();
    let invalid =
        |detail: String| DecodeError::new(ProtocolFailure::InvalidRequest, id.clone(), detail);
    let fields = if has_id {
        REQUEST_FIELDS
    } else {
        NOTIFICATION_FIELDS
    };
    if let Some(unknown) = members
        .iter()
        .map(|(key, _)| key.as_ref())
        .filter(|key| !fields.contains(key))
        .min()
    {
        let error: serde_json::Error = de::Error::unknown_field(unknown, fields);
        return Err(invalid(error.to_string()));
    }
    let Some(jsonrpc) = envelope.jsonrpc else {
        let error: serde_json::Error = de::Error::missing_field("jsonrpc");
        return Err(invalid(error.to_string()));
    };
    if text(jsonrpc).as_deref() != Some("2.0") {
        return Err(invalid("`jsonrpc` is not \"2.0\"".to_owned()));
    }
    let method = match text(method) {
        Some(Cow::Borrowed(method)) => Text::At(range_in(&line, method)),
        Some(Cow::Owned(method)) => Text::Unescaped(method),
        None => return Err(invalid("`method` is a string".to_owned())),
    };
    let params = envelope
        .params
        .filter(|params| params.get() != "null")
        .map(|params| range_in(&line, params.get()));
    drop(members);
    let call = Call {
        line,
        method,
        params,
    };
    match (has_id, id) {
        (false, _) => Ok(Incoming::Notification(call)),
        (true, Some(id)) => Ok(Incoming::Request { id, call }),
        (true, None) => Err(DecodeError::new(
            ProtocolFailure::InvalidRequest,
            None,
            "a request's `id` is a whole number or a string",
        )),
    }
}

/// The envelope's members, the last of each name, as JSON reads them.
#[derive(Clone, Copy, Default)]
struct Envelope<'a> {
    jsonrpc: Option<&'a RawValue>,
    id: Option<&'a RawValue>,
    method: Option<&'a RawValue>,
    params: Option<&'a RawValue>,
    result: Option<&'a RawValue>,
    error: Option<&'a RawValue>,
}

impl<'a> Envelope<'a> {
    fn of(members: &Members<'a>) -> Self {
        let mut envelope = Self::default();
        for (key, value) in members {
            let slot = match key.as_ref() {
                "jsonrpc" => &mut envelope.jsonrpc,
                "id" => &mut envelope.id,
                "method" => &mut envelope.method,
                "params" => &mut envelope.params,
                "result" => &mut envelope.result,
                "error" => &mut envelope.error,
                _ => continue,
            };
            *slot = Some(*value);
        }
        envelope
    }

    /// The answer a reply carries: where its `result` is, or its `error`,
    /// read as the handler's own `handler_failed` when it names no Error
    /// code.
    fn answer(&self, line: &str) -> Option<Result<Range<usize>, ErrorObject>> {
        if let Some(result) = self.result {
            return Some(Ok(range_in(line, result.get())));
        }
        let error = self.error?;
        if let Ok(error) = serde_json::from_str::<ErrorObject>(error.get()) {
            return Some(Err(error));
        }
        let members = match serde_json::from_str::<Shape<'_>>(error.get()) {
            Ok(Shape::Object(members)) => members,
            _ => Members::new(),
        };
        let member = |name: &str| {
            members
                .iter()
                .rev()
                .find(|(key, _)| key == name)
                .map(|(_, value)| *value)
        };
        let message = member("message")
            .and_then(text)
            .map_or_else(|| "the handler failed".to_owned(), Cow::into_owned);
        let code = member("code")
            .and_then(|code| serde_json::from_str::<i64>(code.get()).ok())
            .and_then(|code| i32::try_from(code).ok())
            .unwrap_or(-32000);
        Some(Err(ErrorObject {
            code,
            message,
            data: ErrorData {
                code: "handler_failed".to_owned(),
                retryable: false,
                details: Map::new(),
            },
        }))
    }
}

/// A request's id, when it reads as one: a whole number or a string.
fn request_id(id: &RawValue) -> Option<RequestId> {
    if let Ok(number) = serde_json::from_str::<i64>(id.get()) {
        return Some(RequestId::Number(number));
    }
    text(id).map(|text| RequestId::Text(text.into_owned()))
}

/// The string a JSON value holds: where it is, when it holds no escape.
pub(crate) fn text(value: &RawValue) -> Option<Cow<'_, str>> {
    let json = value.get();
    match json
        .strip_prefix('"')
        .and_then(|json| json.strip_suffix('"'))
    {
        Some(inner) if !inner.contains('\\') => Some(Cow::Borrowed(inner)),
        Some(_) => serde_json::from_str::<String>(json).ok().map(Cow::Owned),
        None => None,
    }
}

/// Where `part`, a slice of `line`, is in it.
fn range_in(line: &str, part: &str) -> Range<usize> {
    let start = (part.as_ptr() as usize) - (line.as_ptr() as usize);
    start..start + part.len()
}

/// An object's members, in order, each value where it is.
pub(crate) type Members<'a> = Vec<(Cow<'a, str>, &'a RawValue)>;

/// What a line's JSON is, its members found when it is an object.
pub(crate) enum Shape<'a> {
    /// An object's members.
    Object(Members<'a>),
    /// An array, which the protocol does not take.
    Batch,
    /// Any other value.
    Scalar,
}

impl<'de> Deserialize<'de> for Shape<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ShapeVisitor)
    }
}

struct ShapeVisitor;

impl<'de> Visitor<'de> for ShapeVisitor {
    type Value = Shape<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut members = Members::with_capacity(map.size_hint().unwrap_or(4));
        while let Some(Key(key)) = map.next_key()? {
            let value = map.next_value::<&RawValue>()?;
            members.push((key, value));
        }
        Ok(Shape::Object(members))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Shape::Batch)
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(Shape::Scalar)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(Shape::Scalar)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(Shape::Scalar)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(Shape::Scalar)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(Shape::Scalar)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(Shape::Scalar)
    }
}

/// Whether `line` nests arrays and objects deeper than `limit`, outside its
/// strings.
fn nesting_exceeds(line: &[u8], limit: usize) -> bool {
    let mut depth = 0_usize;
    let mut at = 0;
    while let Some(&byte) = line.get(at) {
        at += 1;
        match byte {
            b'"' => match string_end(line, at) {
                Some(end) => at = end,
                None => return false,
            },
            b'[' | b'{' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Where the string whose text starts at `at` of `line` ends, just past
/// its closing quote, found a word at a time; none when it never ends.
fn string_end(line: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let found = memchr::memchr2(b'"', b'\\', line.get(at..)?)?;
        let byte = line.get(at + found).copied();
        at += found + 1;
        if byte == Some(b'"') {
            return Some(at);
        }
        // An escape: the byte after it is the string's own.
        at += 1;
    }
}

/// What a decoded message says, as the fuzz target compares it: its id,
/// method and the JSON text of its params, result or error, each under the
/// member that carries it.
#[derive(Debug, PartialEq)]
struct Said {
    id: Option<RequestId>,
    method: Option<String>,
    member: Option<(&'static str, String)>,
}

impl Said {
    fn of(message: &Incoming) -> Self {
        let params = |call: &Call| call.params().map(|json| ("params", json.to_owned()));
        match message {
            Incoming::Request { id, call } => Self {
                id: Some(id.clone()),
                method: Some(call.method().to_owned()),
                member: params(call),
            },
            Incoming::Notification(call) => Self {
                id: None,
                method: Some(call.method().to_owned()),
                member: params(call),
            },
            Incoming::Reply { id, answer } => Self {
                id: Some(id.clone()),
                method: None,
                member: Some(match answer {
                    Ok(result) => ("result", result.json().to_owned()),
                    Err(error) => (
                        "error",
                        serde_json::to_string(error).expect("invariant: an error object encodes"),
                    ),
                }),
            },
        }
    }

    /// The line that says it again.
    fn line(&self) -> Vec<u8> {
        let mut line = br#"{"jsonrpc":"2.0""#.to_vec();
        if let Some(id) = &self.id {
            line.extend_from_slice(b",\"id\":");
            serde_json::to_writer(&mut line, id).expect("invariant: an id encodes");
        }
        if let Some(method) = &self.method {
            line.extend_from_slice(b",\"method\":");
            serde_json::to_writer(&mut line, method).expect("invariant: a method encodes");
        }
        if let Some((name, json)) = &self.member {
            line.extend_from_slice(format!(",\"{name}\":").as_bytes());
            line.extend_from_slice(json.as_bytes());
        }
        line.push(b'}');
        line
    }
}

/// The fuzz target's body (ADR 0067): frames `data` in two chunks under a
/// small limit and decodes each line; a decoded message must encode and
/// decode back to what it says. It panics only on a bug.
pub fn fuzz_framing(data: &[u8]) {
    let mut framer = Framer::new(4096);
    let mut lines = Vec::new();
    let split = data
        .first()
        .map_or(0, |first| usize::from(*first) % (data.len() + 1));
    framer.feed(&data[..split], &mut lines);
    framer.feed(&data[split..], &mut lines);
    lines.extend(framer.finish());
    for frame in lines {
        let Frame::Line(line) = frame else {
            continue;
        };
        assert!(line.len() <= 4096, "a line longer than the framer's limit");
        let Ok(message) = decode(line) else {
            continue;
        };
        let said = Said::of(&message);
        let again = decode(said.line()).map(|message| Said::of(&message));
        assert_eq!(again, Ok(said), "a decoded message reads back as itself");
    }
}

#[cfg(test)]
mod tests;
