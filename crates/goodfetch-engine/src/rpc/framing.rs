//! Newline-delimited JSON-RPC framing, without I/O: bytes in, lines out, and
//! each line decoded into a request, a notification or the reply to a handler
//! request the engine sent. A line longer than
//! [`MAX_MESSAGE_BYTES`] or nested deeper than [`MAX_DEPTH`] ends in its
//! typed limit error (ADR 0067), and the framer keeps no more than the limit
//! in memory while it skips the rest of that line.

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, Notification, ProtocolFailure, Request, RequestId,
};
use serde_json::{Map, Value};

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
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A call.
    Request(Request),
    /// A message that expects no reply.
    Notification(Notification),
    /// The reply to a handler request the engine sent.
    Reply {
        /// The request's id.
        id: RequestId,
        /// Its result, or its failure.
        answer: Result<Value, ErrorObject>,
    },
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

/// Decodes one line into a request or a notification, counting on `work`
/// each whole copy of what it carries (ADR 0140's E1).
pub fn decode(line: &[u8], work: &CountedWork) -> Result<Incoming, DecodeError> {
    if nesting_exceeds(line, MAX_DEPTH) {
        return Err(DecodeError::new(
            ProtocolFailure::MessageTooDeep,
            None,
            format!("a message nests arrays and objects deeper than {MAX_DEPTH}"),
        ));
    }
    let value: Value = serde_json::from_slice(line)
        .map_err(|error| DecodeError::new(ProtocolFailure::ParseError, None, error.to_string()))?;
    let Value::Object(object) = &value else {
        let detail = if value.is_array() {
            "a batch, which the protocol does not take"
        } else {
            "a message is a JSON object"
        };
        return Err(DecodeError::new(
            ProtocolFailure::InvalidRequest,
            None,
            detail,
        ));
    };
    let id = object
        .get("id")
        .and_then(|id| serde_json::from_value::<RequestId>(id.clone()).ok());
    let has_id = object.contains_key("id");
    if !object.contains_key("method")
        && let Some(id) = id.clone()
        && let Some(answer) = reply_answer(object, work)
    {
        return Ok(Incoming::Reply { id, answer });
    }
    if !object.contains_key("method") {
        return Err(DecodeError::new(
            ProtocolFailure::InvalidRequest,
            id,
            "a message to the engine has a `method`",
        ));
    }
    let invalid = |error: serde_json::Error| {
        DecodeError::new(
            ProtocolFailure::InvalidRequest,
            id.clone(),
            error.to_string(),
        )
    };
    work.count(Row::MessageDeepCopies);
    if has_id {
        serde_json::from_value(value.clone())
            .map(Incoming::Request)
            .map_err(invalid)
    } else {
        serde_json::from_value(value.clone())
            .map(Incoming::Notification)
            .map_err(invalid)
    }
}

/// The answer a reply carries: its `result`, or its `error`, read as the
/// handler's own `handler_failed` when it names no Error code.
fn reply_answer(
    object: &Map<String, Value>,
    work: &CountedWork,
) -> Option<Result<Value, ErrorObject>> {
    if let Some(result) = object.get("result") {
        work.count(Row::MessageDeepCopies);
        return Some(Ok(result.clone()));
    }
    let error = object.get("error")?;
    work.count(Row::MessageDeepCopies);
    if let Ok(error) = serde_json::from_value::<ErrorObject>(error.clone()) {
        return Some(Err(error));
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the handler failed")
        .to_owned();
    let code = error
        .get("code")
        .and_then(Value::as_i64)
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

/// Whether `line` nests arrays and objects deeper than `limit`, outside its
/// strings.
fn nesting_exceeds(line: &[u8], limit: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0_usize, false, false);
    for byte in line {
        if in_string {
            match (escaped, byte) {
                (true, _) => escaped = false,
                (false, b'\\') => escaped = true,
                (false, b'"') => in_string = false,
                (false, _) => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
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

/// The fuzz target's body (ADR 0067): frames `data` in two chunks under a
/// small limit and decodes each line; a decoded message must encode and
/// decode back to itself. It panics only on a bug.
pub fn fuzz_framing(data: &[u8]) {
    let work = CountedWork::default();
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
        let Ok(message) = decode(&line, &work) else {
            continue;
        };
        let encoded = match &message {
            Incoming::Request(request) => serde_json::to_vec(request),
            Incoming::Notification(notification) => serde_json::to_vec(notification),
            Incoming::Reply {
                id,
                answer: Ok(result),
            } => serde_json::to_vec(
                &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            ),
            Incoming::Reply {
                id,
                answer: Err(error),
            } => serde_json::to_vec(
                &serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            ),
        }
        .expect("invariant: a decoded message encodes");
        assert_eq!(
            decode(&encoded, &work),
            Ok(message),
            "a decoded message reads back as itself"
        );
    }
}

#[cfg(test)]
mod tests;
