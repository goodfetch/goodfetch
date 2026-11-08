//! The harness's side of one Connection: requests numbered from 1, the
//! answer to each read past the notifications that come meanwhile, and every
//! message exchanged recorded as a case's evidence, `{"from": "client" |
//! "engine", "message": …}`, or `"raw"` for a line that is no JSON, as the
//! Scripted engine records them (ADR 0069), a Daemon's token redacted.

use std::fmt;
use std::time::Duration;

use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, Notification, Request, RequestId, Response, Version,
};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::target::{
    BROWSER_DEADLINE, DEADLINE, EngineTarget, Next, START_DEADLINE, StartError, Started,
};

/// What a kept message holds in place of the Daemon's token: the evidence
/// lands in a result document a Release cell uploads, whose secret scan
/// refuses the token (ADR 0067).
const REDACTED: &str = "[redacted]";

/// `message` with each string that holds `token` holding [`REDACTED`] there.
fn redacted(message: &Value, token: &str) -> Value {
    if token.is_empty() {
        return message.clone();
    }
    match message {
        Value::String(text) if text.contains(token) => Value::String(text.replace(token, REDACTED)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| redacted(item, token)).collect())
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), redacted(value, token)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Why the Connection gave no more answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Gone {
    /// The target closed it.
    Closed,
    /// Nothing came within this many seconds.
    Silent(u64),
    /// Writing or reading failed.
    Failed(String),
}

impl fmt::Display for Gone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("the target closed the Connection"),
            Self::Silent(seconds) => {
                write!(formatter, "the target answered nothing within {seconds} s")
            }
            Self::Failed(reason) => write!(formatter, "the Connection failed: {reason}"),
        }
    }
}

/// The Error code a failure answer names; `null` for a result.
pub(crate) fn code_of(answer: &Value) -> &Value {
    &answer["error"]["data"]["code"]
}

/// Whether `answer` is a navigation's document response with `status`.
pub(crate) fn responded_with(answer: &Value, status: u16) -> bool {
    answer["result"]["response"]["status"] == json!(status)
}

/// An answer, and what came before it that answered something else.
pub(crate) struct Answer {
    /// The response carrying the request's ID.
    pub(crate) message: Value,
    /// Every other message read while waiting for it.
    pub(crate) others: Vec<Value>,
}

/// One open Connection.
pub(crate) struct Connection {
    started: Started,
    next_id: i64,
    exchanged: Vec<Value>,
    /// Whether its `hello` named a surface that acts for a model.
    for_model: bool,
}

impl Connection {
    pub(crate) fn new(started: Started) -> Self {
        Self {
            started,
            next_id: 1,
            exchanged: Vec::new(),
            for_model: false,
        }
    }

    /// Whether it acts for a model, as its `hello` said.
    pub(crate) fn acts_for_model(&self) -> bool {
        self.for_model
    }

    /// Marks it as acting for a model, or not, once its `hello` is sent.
    pub(crate) fn set_for_model(&mut self, for_model: bool) {
        self.for_model = for_model;
    }

    /// The token of the Daemon reached, if any.
    pub(crate) fn token(&self) -> Option<&str> {
        self.started.token.as_deref()
    }

    /// The `hello` this Connection's Client sends from `surface`: the
    /// protocol version the harness speaks, its own Product version and the
    /// token of the Daemon reached, every other field at its default, which
    /// a case sets by struct update.
    pub(crate) fn hello_params(&self, surface: Surface) -> HelloParams {
        HelloParams {
            token: self.token().map(ToOwned::to_owned),
            ..HelloParams::new(
                surface,
                ProductVersion::parse(env!("CARGO_PKG_VERSION"))
                    .expect("invariant: the workspace's version is a Product version"),
            )
        }
    }

    /// Sends a request and returns its ID.
    pub(crate) fn request(&mut self, method: &str, params: &Value) -> Result<i64, Gone> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&Request {
            jsonrpc: Version,
            id: RequestId::Number(id),
            method: method.to_owned(),
            params: Some(params.clone()),
        })?;
        Ok(id)
    }

    /// Sends a notification.
    pub(crate) fn notify(&mut self, method: &str, params: &Value) -> Result<(), Gone> {
        self.send(&Notification {
            jsonrpc: Version,
            method: method.to_owned(),
            params: Some(params.clone()),
        })
    }

    fn send(&mut self, message: &impl Serialize) -> Result<(), Gone> {
        let message = serde_json::to_value(message).expect("invariant: a message serializes");
        let message = &message;
        let line = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
        let kept = match &self.started.token {
            Some(token) => redacted(message, token),
            None => message.clone(),
        };
        self.exchanged
            .push(json!({ "from": "client", "message": kept }));
        self.started
            .write_line(&line)
            .map_err(|error| Gone::Failed(error.to_string()))
    }

    /// The engine's whole answer to `method` with `params`, waiting as long
    /// as a browser's launch may take.
    pub(crate) fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        let id = self.request(method, params)?;
        Ok(self.browser_answer(id)?.message)
    }

    /// `page.goto` of `page` to `url`, and the engine's whole answer.
    pub(crate) fn goto(&mut self, page: &PageOpened, url: &str) -> Result<Value, Gone> {
        self.call(
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )
    }

    /// The answers to the requests `first` and `second`, in whichever order
    /// they come, each within a browser's deadline.
    pub(crate) fn both(&mut self, first: i64, second: i64) -> Result<(Value, Value), Gone> {
        let answer = self.browser_answer(first)?;
        if let Some(early) = answer
            .others
            .into_iter()
            .find(|message| message.get("method").is_none() && message["id"] == json!(second))
        {
            return Ok((answer.message, early));
        }
        Ok((answer.message, self.browser_answer(second)?.message))
    }

    /// The response to the request `id`: the next message that carries the
    /// ID and no method.
    pub(crate) fn answer(&mut self, id: i64) -> Result<Answer, Gone> {
        self.answer_within(id, DEADLINE)
    }

    /// The response to the first request a process just started reads,
    /// which may take longer.
    pub(crate) fn first_answer(&mut self, id: i64) -> Result<Answer, Gone> {
        self.answer_within(id, START_DEADLINE)
    }

    /// The response to a call that launches or closes a browser, which may
    /// take longer.
    pub(crate) fn browser_answer(&mut self, id: i64) -> Result<Answer, Gone> {
        self.answer_within(id, BROWSER_DEADLINE)
    }

    fn answer_within(&mut self, id: i64, deadline: Duration) -> Result<Answer, Gone> {
        let mut others = Vec::new();
        loop {
            let message = self.next_within(deadline)?;
            if message.get("method").is_none() && message.get("id") == Some(&json!(id)) {
                return Ok(Answer { message, others });
            }
            others.push(message);
        }
    }

    /// The next JSON message the target writes, within `deadline`; a line
    /// that is no JSON is recorded and passed over.
    pub(crate) fn next_within(&mut self, deadline: Duration) -> Result<Value, Gone> {
        loop {
            let line = match self.started.next_line(deadline) {
                Next::Line(line) => line,
                Next::Closed => return Err(Gone::Closed),
                Next::Silent => return Err(Gone::Silent(deadline.as_secs())),
                Next::Failed(error) => return Err(Gone::Failed(error.to_string())),
            };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                let raw = match &self.started.token {
                    Some(token) => line.replace(token.as_str(), REDACTED),
                    None => line,
                };
                self.exchanged.push(json!({ "from": "engine", "raw": raw }));
                continue;
            };
            let kept = match &self.started.token {
                Some(token) => redacted(&message, token),
                None => message.clone(),
            };
            self.exchanged
                .push(json!({ "from": "engine", "message": kept }));
            return Ok(message);
        }
    }

    /// Whether the target closes the Connection before writing anything
    /// else.
    pub(crate) fn closes(&mut self) -> bool {
        matches!(self.started.next_line(DEADLINE), Next::Closed)
    }

    /// Answers the engine's request `id` with `result`.
    pub(crate) fn reply(&mut self, id: &Value, result: &Value) -> Result<(), Gone> {
        let id = Self::id_of(id)?;
        self.send(&Response {
            jsonrpc: Version,
            id,
            result: result.clone(),
        })
    }

    /// Answers the engine's request `id` with a failure of the Error code
    /// `code`, of the `program` group and not retryable, as a handler that
    /// threw does.
    pub(crate) fn fail(&mut self, id: &Value, code: &str, message: &str) -> Result<(), Gone> {
        self.fail_with(
            id,
            ErrorGroup::Program,
            ErrorData {
                code: code.to_owned(),
                retryable: false,
                details: Map::new(),
            },
            message,
        )
    }

    /// Answers the engine's request `id` with a failure of `data`'s Error
    /// code in `group`, as a handler that raised an engine error does.
    pub(crate) fn fail_with(
        &mut self,
        id: &Value,
        group: ErrorGroup,
        data: ErrorData,
        message: &str,
    ) -> Result<(), Gone> {
        let id = Self::id_of(id)?;
        self.send(&ErrorResponse {
            jsonrpc: Version,
            id: Some(id),
            error: ErrorObject {
                code: group.json_rpc_class(),
                message: message.to_owned(),
                data,
            },
        })
    }

    /// The ID of an engine's request, which a reply carries back.
    fn id_of(id: &Value) -> Result<RequestId, Gone> {
        serde_json::from_value(id.clone())
            .map_err(|_| Gone::Failed(format!("the engine's request has no ID to answer: {id}")))
    }

    /// Opens a further Connection to the same target, before any `hello`,
    /// its messages kept apart from this one's.
    pub(crate) fn another(&self, target: &EngineTarget) -> Result<Self, StartError> {
        Ok(Self::new(self.started.another(target)?))
    }

    /// Adds `messages`, exchanged on another Connection, to this one's.
    pub(crate) fn keep(&mut self, messages: Vec<Value>) {
        self.exchanged.extend(messages);
    }

    /// The messages exchanged since the last call, oldest first.
    pub(crate) fn take_exchanged(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.exchanged)
    }

    /// Closes the Connection and waits for the target to exit.
    pub(crate) fn close(self) {
        self.started.close();
    }
}
