//! The harness's side of one Connection: requests numbered from 1, the
//! answer to each read past the notifications that come meanwhile, and every
//! message exchanged recorded as a case's evidence, `{"from": "client" |
//! "engine", "message": …}`, or `"raw"` for a line that is no JSON, as the
//! Scripted engine records them (ADR 0069).

use std::fmt;

use serde_json::{Value, json};

use super::target::{DEADLINE, Next, Started};

/// Why the Connection gave no more answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Gone {
    /// The target closed it.
    Closed,
    /// Nothing came before the deadline.
    Silent,
    /// Writing or reading failed.
    Failed(String),
}

impl fmt::Display for Gone {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("the target closed the Connection"),
            Self::Silent => write!(
                formatter,
                "the target answered nothing within {} s",
                DEADLINE.as_secs()
            ),
            Self::Failed(reason) => write!(formatter, "the Connection failed: {reason}"),
        }
    }
}

/// An answer, and what came before it that answered something else.
pub(crate) struct Answer {
    /// The response carrying the request's ID.
    pub(crate) message: Value,
    /// Every other message read while waiting for it.
    pub(crate) others: Vec<Value>,
}

/// One open Connection.
pub(crate) struct Session {
    started: Started,
    next_id: i64,
    exchanged: Vec<Value>,
}

impl Session {
    pub(crate) fn new(started: Started) -> Self {
        Self {
            started,
            next_id: 1,
            exchanged: Vec::new(),
        }
    }

    /// The token of the Daemon reached, if any.
    pub(crate) fn token(&self) -> Option<&str> {
        self.started.token.as_deref()
    }

    /// Sends a request and returns its ID.
    pub(crate) fn request(&mut self, method: &str, params: &Value) -> Result<i64, Gone> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        Ok(id)
    }

    /// Sends a notification.
    pub(crate) fn notify(&mut self, method: &str, params: &Value) -> Result<(), Gone> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, message: &Value) -> Result<(), Gone> {
        let line = serde_json::to_vec(message).expect("invariant: a JSON value serializes");
        self.exchanged
            .push(json!({ "from": "client", "message": message }));
        self.started
            .write_line(&line)
            .map_err(|error| Gone::Failed(error.to_string()))
    }

    /// The response to the request `id`: the next message that carries the
    /// ID and no method.
    pub(crate) fn answer(&mut self, id: i64) -> Result<Answer, Gone> {
        let mut others = Vec::new();
        loop {
            let line = match self.started.next_line() {
                Next::Line(line) => line,
                Next::Closed => return Err(Gone::Closed),
                Next::Silent => return Err(Gone::Silent),
                Next::Failed(error) => return Err(Gone::Failed(error.to_string())),
            };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                self.exchanged
                    .push(json!({ "from": "engine", "raw": line }));
                continue;
            };
            self.exchanged
                .push(json!({ "from": "engine", "message": message }));
            if message.get("method").is_none() && message.get("id") == Some(&json!(id)) {
                return Ok(Answer { message, others });
            }
            others.push(message);
        }
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
