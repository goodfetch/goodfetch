//! Why a command failed, as typed variants (ADR 0127): the browser's
//! refusal, classified here from its error's code and message, the only
//! place that reads them; a command the browser did not answer in time; or
//! a browser that is gone. The browser's own message is kept for diagnosis
//! in [`PartyText`], which has no accessor.

use std::fmt;

/// The browser's own text, kept for logs and never matched on outside this
/// module.
#[derive(Clone, PartialEq, Eq)]
pub struct PartyText(String);

impl PartyText {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl fmt::Debug for PartyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl fmt::Display for PartyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// What a refusal of the browser's says, by its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// The page or Session the command named is gone: closed, or its
    /// renderer crashed.
    TargetGone,
    /// The URL is not one the browser can navigate to.
    InvalidUrl,
    /// A message this table does not know yet: logged at `warn`, and the
    /// call that met it raises its own generic code (ADR 0127).
    Unrecognized,
}

/// Why a command failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Failure {
    /// The browser answered with an error.
    #[error("the browser refused `{method}` ({code}): {message}")]
    Refused {
        /// The command's method.
        method: &'static str,
        /// What the refusal says.
        refusal: Refusal,
        /// The error's JSON-RPC code.
        code: i64,
        /// The browser's message.
        message: PartyText,
    },
    /// The browser did not answer within the command's timeout.
    #[error("the browser did not answer `{method}` in time")]
    TimedOut {
        /// The command's method.
        method: &'static str,
    },
    /// The connection to the browser ended: its process exited or closed
    /// its end of the pipe.
    #[error("the browser's connection ended")]
    Disconnected,
    /// The browser's answer is not what the command returns.
    #[error("the browser's answer to `{method}` is not what it returns: {message}")]
    Unreadable {
        /// The command's method.
        method: &'static str,
        /// What did not read.
        message: PartyText,
    },
}

/// The messages the browser refuses with, each with its meaning. Chromium's
/// texts, as its protocol handlers word them.
const REFUSALS: [(&str, Refusal); 5] = [
    ("Target closed", Refusal::TargetGone),
    ("Session with given id not found", Refusal::TargetGone),
    ("No target with given id found", Refusal::TargetGone),
    ("Not attached to an active page", Refusal::TargetGone),
    ("Cannot navigate to invalid URL", Refusal::InvalidUrl),
];

impl Failure {
    /// The failure an error object of the browser's answer to `method` is.
    pub(crate) fn refused(method: &'static str, error: &serde_json::Value) -> Self {
        let code = error
            .get("code")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default();
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let refusal = REFUSALS
            .iter()
            .find(|(text, _)| message.starts_with(text))
            .map_or(Refusal::Unrecognized, |(_, refusal)| *refusal);
        if refusal == Refusal::Unrecognized {
            tracing::warn!(
                method,
                code,
                message,
                "an unrecognized refusal of the browser's"
            );
        }
        Self::Refused {
            method,
            refusal,
            code,
            message: PartyText::new(message),
        }
    }
}

#[cfg(test)]
mod tests;
