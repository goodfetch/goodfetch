//! The same API without `async` (ADR 0081), generated from the same catalog as Python's sync API.
//!
//! Each call hands its work to the connection's runtime and waits on a channel. It never nests a
//! runtime, so it also works inside an async runtime, where it only blocks that thread. A call is a
//! builder ended by [`Call::send`], as in `reqwest::blocking`; a stream is an [`Iterator`]. Handles
//! are cheap clones, safe to use from any thread. Helpers that answer requests run handlers on the
//! `tasks` module's tracked blocking threads (ADR 0060), so blocking handlers are
//! `Fn + Send + Sync + 'static`. Only a few calls are sketched here; the rest mirror the async API.

use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use crate::concurrency::events;
use crate::results::Error;
use crate::page::Response;
use crate::time::Limits;

/// One blocking call: set its timeout, then [`Call::send`] it.
#[must_use = "a call does nothing until it is sent"]
pub struct Call<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Call<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Call(..)")
    }
}

impl<T> Call<T> {
    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// Sends the call and blocks until it ends.
    pub fn send(self) -> Result<T, Error> {
        unimplemented!()
    }
}

/// What the engine sends on its own, read in order; dropping it unsubscribes without blocking.
pub struct EventIter<T> {
    _item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for EventIter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EventIter(..)")
    }
}

impl<T> Iterator for EventIter<T> {
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        unimplemented!()
    }
}

/// Starting a Run, not sent until [`Start::send`].
#[derive(Debug)]
#[must_use = "a Run does not start until this is sent"]
pub struct Start {
    _private: (),
}

impl Start {
    /// Names the Run for good: a start with an existing Run ID resumes the Run if it was
    /// interrupted, or returns it as it ended (ADR 0102); a new ID starts a new Run.
    ///
    /// As [`crate::Start::run_id`]'s.
    pub fn run_id(self, run_id: impl Into<String>) -> Self {
        let _ = run_id.into();
        unimplemented!()
    }

    /// How long the whole Run may last.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// The Run's Limits.
    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        unimplemented!()
    }

    /// Starts the Run and blocks until it has started.
    pub fn send(self) -> Result<Run, Error> {
        unimplemented!()
    }
}

/// A Run, blocking.
#[derive(Debug, Clone)]
pub struct Run {
    _private: (),
}

impl Run {
    /// Opens a page.
    pub fn new_page(&self) -> Call<Page> {
        unimplemented!()
    }

    /// Leaving the scope or `close()` stops the Run with a Checkpoint; `finish()` ends it.
    ///
    /// Stops the Run if unfinished. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A page, blocking.
#[derive(Debug, Clone)]
pub struct Page {
    _private: (),
}

impl Page {
    /// Placeholder for #472's navigation.
    pub fn goto(&self, url: &str) -> Call<Option<Response>> {
        let _ = url;
        unimplemented!()
    }

    /// The event's iterator, subscribed when called.
    pub fn events<E: events::PageEvent>(&self, event: E) -> EventIter<E::Data> {
        let _ = event;
        unimplemented!()
    }

    /// Closes the tab. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// Any blocking connection, as [`crate::Connection`] (ADR 0099): what
/// `goodfetch_collie::blocking::Collie` and `goodfetch_foxhound::blocking::Foxhound` share.
pub trait Connection {
    /// The engine reached, fixed when the connection opened.
    fn engine(&self) -> &crate::EngineInfo;

    /// The install's Saved identities.
    fn identities(&self) -> crate::Identities;

    /// The raw catalog call; set its parameters with [`RawCall::params`], then [`RawCall::send`] it.
    fn call(&self, operation: &str) -> RawCall;

    /// The connection's effective settings, with the layer that set each.
    fn explain_settings(&self, keys: &[crate::SettingKey]) -> Call<Vec<crate::ExplainedSetting>>;

    /// Closes the connection for every clone. Idempotent.
    fn close(&self) -> Call<()>;

    /// Starts a Run, as [`crate::Connection::start`].
    fn start(&self) -> Start;
}

/// One blocking raw catalog call: set its parameters and timeout, then [`RawCall::send`] it.
#[must_use = "a call does nothing until it is sent"]
pub struct RawCall {
    _private: (),
}

impl fmt::Debug for RawCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RawCall(..)")
    }
}

impl RawCall {
    /// The operation's parameters: the wire's JSON, in snake_case.
    pub fn params(self, params: crate::JsonObject) -> Self {
        let _ = params;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// Sends the call and waits for its result.
    pub fn send(self) -> Result<serde_json::Value, Error> {
        unimplemented!()
    }
}
