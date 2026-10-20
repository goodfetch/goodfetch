//! The Scripted engine's player (ADR 0069): plays one script to a Client
//! over a pair of byte streams, newline-delimited JSON-RPC each way, and
//! records every message it reads and writes. It matches each message the
//! Client sends on its method and the canonical JSON of its params, answers
//! with the ID of the request the step before matched, and fails on a
//! message no step expects and on a step left unused. The
//! `goodfetch-scripted-engine` executable plays it over its own stdio.

use std::fmt;
use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::script::{Call, Reply, Script, Sent, Step};

/// Where an Attached engine started as an SDK starts one,
/// `goodfetch-scripted-engine attached-engine`, finds its script: under
/// the Goodfetch home its environment names.
pub const ATTACHED_SCRIPT: &str = "scripted-engine/script.json";

/// Where that engine writes its record, under the same home: one JSON line
/// per message, `{"from": "client" | "engine", "message": …}`, or `"raw"`
/// for a line that is not JSON.
pub const ATTACHED_RECORD: &str = "scripted-engine/record.jsonl";

/// How a script that played to its end ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The Client closed its end after the last step.
    Finished,
    /// The script closed the connection.
    Closed,
    /// The script exits its process with this code.
    Exit(u8),
}

/// Plays `script` to the Client that writes `client` and reads `engine`,
/// writing each message read or written to `record`.
///
/// # Errors
///
/// The Client sent a message no step expects, or closed its end with a step
/// unused, or a stream failed.
pub fn play(
    script: &Script,
    client: impl BufRead,
    engine: impl Write,
    record: impl Write,
) -> Result<Ending, PlayError> {
    let mut player = Player {
        client: client.lines(),
        engine,
        record,
        answering: None,
        awaiting: None,
        next_id: 1,
    };
    let steps = &script.steps;
    for (index, step) in steps.iter().enumerate() {
        let ending = player.step(index, step)?;
        if let Some(ending) = ending {
            if index + 1 < steps.len() {
                return Err(PlayError::UnusedStep { step: index + 1 });
            }
            return Ok(ending);
        }
    }
    player.until_closed("the Client to close its end after the last step")?;
    Ok(Ending::Finished)
}

/// The state of one play.
struct Player<C, E, R> {
    client: io::Lines<C>,
    engine: E,
    record: R,
    /// The ID of the request the last `expect` matched, which `reply`
    /// answers.
    answering: Option<Value>,
    /// The ID of the request the last `send` sent, which `await_reply`
    /// matches.
    awaiting: Option<i64>,
    next_id: i64,
}

impl<C: BufRead, E: Write, R: Write> Player<C, E, R> {
    /// Plays one step; an ending step ends the play.
    fn step(&mut self, index: usize, step: &Step) -> Result<Option<Ending>, PlayError> {
        match step {
            Step::Expect(expectation) => {
                let message = self.read(index)?;
                let wanted = Call {
                    method: expectation.method.clone(),
                    params: expectation.params.clone(),
                };
                let matches = message.get("result").is_none()
                    && message.get("error").is_none()
                    && message.get("method").and_then(Value::as_str) == Some(&wanted.method)
                    && canonical(message.get("params")) == canonical(wanted.params.as_ref());
                if !matches {
                    return Err(unexpected(index, &describe(&wanted), &message));
                }
                self.answering = message.get("id").cloned();
            }
            Step::Reply(reply) => {
                let id = self
                    .answering
                    .take()
                    .ok_or(PlayError::NothingToAnswer { step: index })?;
                let mut message = json!({ "jsonrpc": "2.0", "id": id });
                let (key, value) = reply_parts(reply);
                message[key] = value.clone();
                self.write(&message)?;
            }
            Step::Send(Sent::Request(call)) => {
                let id = self.next_id;
                self.next_id += 1;
                let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": call.method });
                if let Some(params) = &call.params {
                    message["params"] = params.clone();
                }
                self.write(&message)?;
                self.awaiting = Some(id);
            }
            Step::Send(Sent::Notification(call)) => {
                let mut message = json!({ "jsonrpc": "2.0", "method": call.method });
                if let Some(params) = &call.params {
                    message["params"] = params.clone();
                }
                self.write(&message)?;
            }
            Step::AwaitReply(reply) => {
                let id = self
                    .awaiting
                    .take()
                    .ok_or(PlayError::NothingToAnswer { step: index })?;
                let message = self.read(index)?;
                let (key, value) = reply_parts(reply);
                let matches = message.get("method").is_none()
                    && message.get("id") == Some(&json!(id))
                    && canonical(message.get(key)) == canonical(Some(value));
                if !matches {
                    let wanted = format!(
                        "the answer {{\"{key}\": {}}} to request {id}",
                        canonical(Some(value))
                    );
                    return Err(unexpected(index, &wanted, &message));
                }
            }
            Step::Stall {} => {
                self.until_closed("no message until the Client gives up")?;
                return Ok(Some(Ending::Finished));
            }
            Step::Close {} => return Ok(Some(Ending::Closed)),
            Step::Raw(raw) => {
                self.engine
                    .write_all(raw.text.as_bytes())
                    .and_then(|()| self.engine.flush())
                    .map_err(PlayError::ClientGone)?;
                self.note(&json!({ "from": "engine", "raw": raw.text }))?;
            }
            Step::Exit(exit) => return Ok(Some(Ending::Exit(exit.code))),
        }
        Ok(None)
    }

    /// The Client's next message, for the step `index`.
    fn read(&mut self, index: usize) -> Result<Value, PlayError> {
        let Some(line) = self.client.next() else {
            return Err(PlayError::UnusedStep { step: index });
        };
        let line = line.map_err(PlayError::Read)?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            self.note(&json!({ "from": "client", "raw": line }))?;
            return Err(PlayError::UnexpectedMessage {
                step: Some(index),
                expected: "a JSON-RPC message".to_owned(),
                received: line.escape_debug().to_string(),
            });
        };
        self.note(&json!({ "from": "client", "message": message }))?;
        Ok(message)
    }

    /// Reads until the Client closes its end, failing on any message.
    fn until_closed(&mut self, expected: &str) -> Result<(), PlayError> {
        let Some(line) = self.client.next() else {
            return Ok(());
        };
        let line = line.map_err(PlayError::Read)?;
        let entry = serde_json::from_str::<Value>(&line).map_or_else(
            |_| json!({ "from": "client", "raw": line }),
            |message| json!({ "from": "client", "message": message }),
        );
        self.note(&entry)?;
        Err(PlayError::UnexpectedMessage {
            step: None,
            expected: expected.to_owned(),
            received: line.escape_debug().to_string(),
        })
    }

    fn write(&mut self, message: &Value) -> Result<(), PlayError> {
        let mut line = message.to_string();
        line.push('\n');
        self.engine
            .write_all(line.as_bytes())
            .and_then(|()| self.engine.flush())
            .map_err(PlayError::ClientGone)?;
        self.note(&json!({ "from": "engine", "message": message }))
    }

    fn note(&mut self, entry: &Value) -> Result<(), PlayError> {
        let mut line = entry.to_string();
        line.push('\n');
        self.record
            .write_all(line.as_bytes())
            .and_then(|()| self.record.flush())
            .map_err(PlayError::Record)
    }
}

/// The key and the value an answer takes on the wire.
fn reply_parts(reply: &Reply) -> (&'static str, &Value) {
    match reply {
        Reply::Result(value) => ("result", value),
        Reply::Error(value) => ("error", value),
    }
}

/// The canonical JSON (RFC 8785) of a value, or `absent`.
fn canonical(value: Option<&Value>) -> String {
    value.map_or_else(
        || "absent".to_owned(),
        |value| {
            serde_json_canonicalizer::to_string(value)
                .expect("invariant: a JSON value read from text is canonicalized")
        },
    )
}

fn describe(call: &Call) -> String {
    format!(
        "`{}` with params {}",
        call.method,
        canonical(call.params.as_ref())
    )
}

fn unexpected(step: usize, expected: &str, message: &Value) -> PlayError {
    PlayError::UnexpectedMessage {
        step: Some(step),
        expected: expected.to_owned(),
        received: canonical(Some(message)),
    }
}

/// Why a play failed. Its `Display` opens with its code, such as
/// `unexpected_message`, which a test matches on.
#[derive(Debug)]
#[non_exhaustive]
pub enum PlayError {
    /// The Client sent a message no step expects.
    UnexpectedMessage {
        /// The step that expected another, or none after the last.
        step: Option<usize>,
        /// What the step expected.
        expected: String,
        /// What came, as canonical JSON or the escaped line.
        received: String,
    },
    /// The Client closed its end, or the script ended, with this step
    /// unused.
    UnusedStep {
        /// The first step left unused.
        step: usize,
    },
    /// A `reply` or `await_reply` step has no request before it to answer.
    NothingToAnswer {
        /// The step.
        step: usize,
    },
    /// Writing to the Client failed: it closed its end.
    ClientGone(io::Error),
    /// Reading from the Client failed.
    Read(io::Error),
    /// Writing the record failed.
    Record(io::Error),
}

impl PlayError {
    /// The failure's code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnexpectedMessage { .. } => "unexpected_message",
            Self::UnusedStep { .. } => "unused_step",
            Self::NothingToAnswer { .. } => "nothing_to_answer",
            Self::ClientGone(_) => "client_gone",
            Self::Read(_) => "read_failed",
            Self::Record(_) => "record_failed",
        }
    }
}

impl fmt::Display for PlayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            Self::UnexpectedMessage {
                step: Some(step),
                expected,
                received,
            } => write!(
                formatter,
                "{code}: step {step} expects {expected}, and the Client sent {received}"
            ),
            Self::UnexpectedMessage {
                step: None,
                expected,
                received,
            } => write!(
                formatter,
                "{code}: the script expects {expected}, and the Client sent {received}"
            ),
            Self::UnusedStep { step } => {
                write!(formatter, "{code}: step {step} was left unused")
            }
            Self::NothingToAnswer { step } => write!(
                formatter,
                "{code}: step {step} answers a request, and none came before it"
            ),
            Self::ClientGone(error) | Self::Read(error) | Self::Record(error) => {
                write!(formatter, "{code}: {error}")
            }
        }
    }
}

impl std::error::Error for PlayError {}
