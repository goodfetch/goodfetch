//! The Connection as a case with several calls in flight reads it: every
//! message read while waiting for another is kept for whoever waits for it,
//! so no call's answer is lost to another's wait, and the events one
//! subscription delivers are gathered apart.

use std::time::Duration;

use goodfetch_protocol::events::EVENTS;
use serde_json::{Value, json};

use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::BROWSER_DEADLINE;

/// One Connection read by a case.
pub(super) struct Exchange<'connection> {
    pub(super) connection: &'connection mut Connection,
    /// The messages read while waiting for another, in order.
    kept: Vec<Value>,
    /// The subscription whose events are gathered, if any.
    subscription: Option<i64>,
    /// The events it delivered so far.
    events: Vec<Value>,
}

impl<'connection> Exchange<'connection> {
    pub(super) fn new(connection: &'connection mut Connection) -> Self {
        Self {
            connection,
            kept: Vec::new(),
            subscription: None,
            events: Vec::new(),
        }
    }

    /// Gathers the events the subscription `subscription` delivers.
    pub(super) fn with_events(mut self, subscription: i64) -> Self {
        self.subscription = Some(subscription);
        self
    }

    pub(super) fn request(&mut self, method: &str, params: &Value) -> Result<i64, Gone> {
        self.connection.request(method, params)
    }

    /// The next message the Connection reads within `deadline`, unless it is
    /// a batch of the gathered subscription's events, which is gathered.
    fn read(&mut self, deadline: Duration) -> Result<Option<Value>, Gone> {
        let message = self.connection.next_within(deadline)?;
        if message["method"] == json!(EVENTS)
            && self
                .subscription
                .is_some_and(|subscription| message["params"]["id"] == json!(subscription))
        {
            self.events.extend(
                message["params"]["events"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
            return Ok(None);
        }
        Ok(Some(message))
    }

    /// The next message other than the gathered events, a kept one first.
    pub(super) fn next(&mut self) -> Result<Value, Gone> {
        if !self.kept.is_empty() {
            return Ok(self.kept.remove(0));
        }
        loop {
            if let Some(message) = self.read(BROWSER_DEADLINE)? {
                return Ok(message);
            }
        }
    }

    /// Keeps `messages` for whoever waits for them, in order.
    pub(super) fn keep_all(&mut self, messages: Vec<Value>) {
        self.kept.extend(messages);
    }

    /// The answer to the request `id`, read within `deadline`, every other
    /// message kept.
    pub(super) fn answer_within(&mut self, id: i64, deadline: Duration) -> Result<Value, Gone> {
        if let Some(at) = self.kept.iter().position(|message| is_answer(message, id)) {
            return Ok(self.kept.remove(at));
        }
        loop {
            match self.read(deadline)? {
                Some(message) if is_answer(&message, id) => return Ok(message),
                Some(message) => self.kept.push(message),
                None => {}
            }
        }
    }

    pub(super) fn answer(&mut self, id: i64) -> Result<Value, Gone> {
        self.answer_within(id, BROWSER_DEADLINE)
    }

    /// The answer to the request `id` if it comes within `quiet`.
    pub(super) fn answered_within(
        &mut self,
        id: i64,
        quiet: Duration,
    ) -> Result<Option<Value>, Gone> {
        match self.answer_within(id, quiet) {
            Ok(answer) => Ok(Some(answer)),
            Err(Gone::Silent(_)) => Ok(None),
            Err(gone) => Err(gone),
        }
    }

    pub(super) fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        let id = self.request(method, params)?;
        self.answer(id)
    }

    /// The data of the first gathered event `name` whose data `holds`,
    /// read until it arrives, every other message kept.
    pub(super) fn until(
        &mut self,
        name: &str,
        holds: impl Fn(&Value) -> bool,
    ) -> Result<Value, Gone> {
        loop {
            if let Some(event) = self
                .events
                .iter()
                .find(|event| event["event"] == json!(name) && holds(&event["data"]))
            {
                return Ok(event["data"].clone());
            }
            if let Some(message) = self.read(BROWSER_DEADLINE)? {
                self.kept.push(message);
            }
        }
    }

    /// Reads until the target stays silent for `quiet`, gathering events
    /// and keeping every other message.
    pub(super) fn idle(&mut self, quiet: Duration) -> Result<(), Gone> {
        loop {
            match self.read(quiet) {
                Ok(Some(message)) => self.kept.push(message),
                Ok(None) => {}
                Err(Gone::Silent(_)) => return Ok(()),
                Err(gone) => return Err(gone),
            }
        }
    }

    /// How many events `name` were gathered.
    pub(super) fn count(&self, name: &str) -> usize {
        self.events
            .iter()
            .filter(|event| event["event"] == json!(name))
            .count()
    }
}

/// Whether `message` is the answer to the request `id`.
fn is_answer(message: &Value, id: i64) -> bool {
    message.get("method").is_none() && message["id"] == json!(id)
}
