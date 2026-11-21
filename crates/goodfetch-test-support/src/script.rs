//! The Scripted engine's script format (ADR 0069): a description and the
//! steps one exchange of protocol messages takes, each message held as a
//! plain JSON value, so the format names no Goodfetch type. Scripts live
//! once, in `test-support/fixtures/engine-scripts/`, grouped by capability;
//! every player plays them, and a Quick-suite check validates each message
//! step against the committed JSON Schema bundle. `xtask codegen` commits
//! the format's own schema there, as `script.schema.json`.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads a script file outside any async worker (ADR 0062)"
)]

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The folder of the corpus, `test-support/fixtures/engine-scripts/`.
pub const ENGINE_SCRIPTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test-support/fixtures/engine-scripts"
);

/// The path of the corpus's script `name`, such as
/// `hello/protocol-version-unsupported.json`.
pub fn engine_script(name: &str) -> PathBuf {
    Path::new(ENGINE_SCRIPTS).join(name)
}

/// One exchange of protocol messages a Scripted engine plays to a Client.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// What the exchange shows, for people.
    pub description: String,
    /// The steps, played in order; the player fails on a message no step
    /// expects and on a step left unused.
    pub steps: Vec<Step>,
}

impl Script {
    /// Reads the script at `path`.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or holds no script of this format.
    pub fn read(path: &Path) -> Result<Self, ScriptError> {
        let text = std::fs::read(path).map_err(|source| ScriptError::Unreadable {
            path: path.to_owned(),
            source,
        })?;
        serde_json::from_slice(&text).map_err(|source| ScriptError::Invalid {
            path: path.to_owned(),
            source,
        })
    }
}

/// One step of a script.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// The next message from the Client: a request or a notification of
    /// this method, whose params are these, compared as canonical JSON.
    Expect(Expectation),
    /// The engine's answer to the request the last `expect` of a request
    /// matched, which carries that request's ID; an expected notification
    /// between them, such as the `$/cancel` of that request, leaves it to
    /// answer. A script never writes an ID.
    Reply(Reply),
    /// A message the engine starts: a request to one of the Client's
    /// handlers, or a notification such as an event.
    Send(Sent),
    /// The Client's answer to the request the last `send` sent, compared
    /// as canonical JSON.
    AwaitReply(Reply),
    /// The engine answers nothing more and waits until the Client closes
    /// its end, as an SDK's own short timeout makes it do; a message that
    /// comes meanwhile is unexpected.
    Stall {},
    /// The engine closes the connection.
    Close {},
    /// Bytes the engine writes that are not JSON-RPC, which the check skips.
    Raw(Raw),
    /// The engine's process exits at once with this code, as a crash does,
    /// first writing its crash output to its stderr when the step has one.
    Exit(Exit),
}

/// What an `expect` step matches.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    /// The method, such as `hello` or `engine.info`.
    pub method: String,
    /// The params, absent when the message carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// An answer to a request: its result, or its failure as the wire's error
/// object (`code`, `message` and `data` with the Error code).
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    /// What the call answered.
    Result(Value),
    /// The failure.
    Error(Value),
}

/// A message the engine starts.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Sent {
    /// A request, whose ID the player chooses, answered by `await_reply`.
    Request(Call),
    /// A notification, such as an event.
    Notification(Call),
}

/// A method and its params.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Call {
    /// The method.
    pub method: String,
    /// The params, absent when the message carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Bytes that are not JSON-RPC.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Raw {
    /// The text written as it is, with no newline added.
    pub text: String,
}

/// How the engine's process exits.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Exit {
    /// The exit code.
    pub code: u8,
    /// What the engine writes to its stderr just before it exits, as a
    /// crash prints its output; nothing when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<StderrLines>,
}

/// Lines an exiting engine writes to its stderr: one line written `times`
/// times, so a script can write more than an SDK keeps of it (64 KiB, ADR
/// 0078) without holding all of it.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct StderrLines {
    /// The line, written with a newline after it.
    pub line: String,
    /// How many times it is written.
    pub times: u16,
}

impl StderrLines {
    /// The text written: `line` and a newline, `times` times.
    #[must_use]
    pub fn text(&self) -> String {
        let mut line = self.line.clone();
        line.push('\n');
        line.repeat(usize::from(self.times))
    }
}

/// Why a script cannot be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScriptError {
    /// The file cannot be read.
    #[error("the script {} cannot be read: {source}", path.display())]
    Unreadable {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The file holds no script of this format.
    #[error("the script {} is invalid: {source}", path.display())]
    Invalid {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
}

#[cfg(test)]
mod tests;
