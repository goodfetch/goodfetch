//! The Scripted engine's player (ADR 0069): plays one script to a Client
//! over a pair of byte streams, newline-delimited JSON-RPC each way, and
//! records every message it reads and writes. It matches each message the
//! Client sends on its method and the canonical JSON of its params, answers
//! with the ID of the request the last `expect` of a request matched, and
//! fails on a message no step expects and on a step left unused. The
//! `goodfetch-scripted-engine` executable plays it over its own stdio.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use crate::script::{Call, Exit, Reply, Script, Sent, Step};
use crate::strict_stdout::is_json_rpc;

/// Where an Attached engine started as an SDK starts one,
/// `goodfetch-scripted-engine attached-engine`, finds its script: under
/// the Goodfetch home its environment names.
pub const ATTACHED_SCRIPT: &str = "scripted-engine/script.json";

/// Where that engine writes its record, under the same home: one JSON line
/// per message, `{"from": "client" | "engine", "message": …}`, or `"raw"`
/// for a line that is not JSON.
pub const ATTACHED_RECORD: &str = "scripted-engine/record.jsonl";

/// Where that engine writes, before it plays, the environment it was
/// started with, under the same home: one JSON object of each variable's
/// name and value, a part that is not Unicode replaced by U+FFFD, so a test
/// sees what its SDK passed (ADR 0083).
pub const ATTACHED_ENVIRONMENT: &str = "scripted-engine/environment.json";

/// How a script that played to its end ends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    /// The Client closed its end after the last step.
    Finished,
    /// The script closed the connection.
    Closed,
    /// The script exits its process as this step says.
    Exit(Exit),
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
    played(script, client, engine, record, None)
}

/// Plays `script` as [`play`] does, over streams that `relink` drops and
/// moves to the next Client to connect, so a `drop` step plays the steps
/// after it to that Client.
///
/// # Errors
///
/// As [`play`], or `relink` failed.
pub fn play_relinking(
    script: &Script,
    client: impl BufRead,
    engine: impl Write,
    record: impl Write,
    mut relink: impl FnMut() -> io::Result<()>,
) -> Result<Ending, PlayError> {
    played(script, client, engine, record, Some(&mut relink))
}

fn played(
    script: &Script,
    client: impl BufRead,
    engine: impl Write,
    record: impl Write,
    relink: Option<&mut dyn FnMut() -> io::Result<()>>,
) -> Result<Ending, PlayError> {
    let mut player = Player {
        client: client.lines(),
        engine,
        record,
        relink,
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
struct Player<'a, C, E, R> {
    client: io::Lines<C>,
    engine: E,
    record: R,
    /// Drops the Client and moves the streams to the next, for a `drop`
    /// step; none when the streams take no other Client.
    relink: Option<&'a mut dyn FnMut() -> io::Result<()>>,
    /// The ID of the request the last `expect` of a request matched, which
    /// `reply` answers.
    answering: Option<Value>,
    /// The ID of the request the last `send` sent, which `await_reply`
    /// matches.
    awaiting: Option<i64>,
    next_id: i64,
}

impl<C: BufRead, E: Write, R: Write> Player<'_, C, E, R> {
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
                // A notification, such as a `$/cancel`, leaves the request
                // matched before it to answer.
                if let Some(id) = message.get("id") {
                    self.answering = Some(id.clone());
                }
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
            Step::Drop {} => {
                let relink = self
                    .relink
                    .as_mut()
                    .ok_or(PlayError::DropUnsupported { step: index })?;
                relink().map_err(PlayError::ClientGone)?;
                // The next Client has sent nothing, and been sent nothing.
                self.answering = None;
                self.awaiting = None;
            }
            Step::Raw(raw) => {
                self.engine
                    .write_all(raw.text.as_bytes())
                    .and_then(|()| self.engine.flush())
                    .map_err(PlayError::ClientGone)?;
                self.note(&json!({ "from": "engine", "raw": raw.text }))?;
            }
            Step::Exit(exit) => return Ok(Some(Ending::Exit(exit.clone()))),
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
        // The real engine refuses what is not one JSON-RPC 2.0 message: a
        // missing or other `jsonrpc`, a reply with both `result` and `error`.
        if !is_json_rpc(&message) {
            return Err(unexpected(index, "a JSON-RPC 2.0 message", &message));
        }
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
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlayError {
    /// The Client sent a message no step expects.
    #[error("unexpected_message: {} expects {expected}, and the Client sent {received}", expecting(*step))]
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
    #[error("unused_step: step {step} was left unused")]
    UnusedStep {
        /// The first step left unused.
        step: usize,
    },
    /// A `reply` or `await_reply` step has no request before it to answer.
    #[error("nothing_to_answer: step {step} answers a request, and none came before it")]
    NothingToAnswer {
        /// The step.
        step: usize,
    },
    /// A `drop` step, played where no other Client can connect: only the
    /// `websocket` mode takes one.
    #[error("drop_unsupported: step {step} drops the Client, and this mode takes no other")]
    DropUnsupported {
        /// The step.
        step: usize,
    },
    /// Writing to the Client failed: it closed its end.
    #[error("client_gone: {0}")]
    ClientGone(io::Error),
    /// Reading from the Client failed.
    #[error("read_failed: {0}")]
    Read(io::Error),
    /// Writing the record failed.
    #[error("record_failed: {0}")]
    Record(io::Error),
}

impl PlayError {
    /// The failure's code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnexpectedMessage { .. } => "unexpected_message",
            Self::UnusedStep { .. } => "unused_step",
            Self::NothingToAnswer { .. } => "nothing_to_answer",
            Self::DropUnsupported { .. } => "drop_unsupported",
            Self::ClientGone(_) => "client_gone",
            Self::Read(_) => "read_failed",
            Self::Record(_) => "record_failed",
        }
    }
}

/// Which step of the script expected a message: `step <n>`, or the script
/// itself after its last step.
fn expecting(step: Option<usize>) -> String {
    step.map_or_else(|| "the script".to_owned(), |step| format!("step {step}"))
}

#[cfg(test)]
mod tests;
